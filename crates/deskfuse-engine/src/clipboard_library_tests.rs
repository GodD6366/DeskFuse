//! Regression cases exercise the old store, interrupted jobs and causal merges.
use super::*;

fn text(library: &Library, value: &str) -> u64 {
    library
        .ingest(
            &PasteboardSnapshot {
                text: Some(value.into()),
                ..Default::default()
            },
            "local",
        )
        .unwrap()
        .unwrap()
}
fn page(library: &Library, query: &str) -> Value {
    serde_json::from_str(
        &library
            .query(&json!({"op":"items","query":query,"include_total":false}).to_string())
            .unwrap(),
    )
    .unwrap()
}
fn finish_metadata(library: &Library) {
    while !library.metadata_ready.load(Ordering::Acquire) {
        library.storage_step().unwrap();
    }
}

#[test]
fn blob_migration_uses_pending_index_and_picks_up_later_writes() {
    let library = Library::open().unwrap();
    assert!(!library.migrate_blob_to_fs_step().unwrap());
    {
        let db = library.writer().unwrap();
        db.execute("INSERT INTO cl_blobs VALUES('small',zeroblob(65536))", [])
            .unwrap();
        let plan: Vec<String> = db.prepare("EXPLAIN QUERY PLAN SELECT hash FROM cl_blobs WHERE length(bytes) > 65536 ORDER BY length(bytes) DESC LIMIT 1")
            .unwrap().query_map([], |row| row.get(3)).unwrap()
            .collect::<std::result::Result<_, _>>().unwrap();
        assert!(
            plan.iter()
                .any(|line| line.contains("cl_blob_migration_pending")),
            "{plan:?}"
        );
        assert!(
            !plan.iter().any(|line| line.contains("TEMP B-TREE")),
            "{plan:?}"
        );
    }
    assert!(!library.migrate_blob_to_fs_step().unwrap());
    for hash in ["abcdef01", "abcdef02"] {
        library
            .writer()
            .unwrap()
            .execute("INSERT INTO cl_blobs VALUES(?1,zeroblob(70000))", [hash])
            .unwrap();
        assert!(library.migrate_blob_to_fs_step().unwrap());
        assert_eq!(
            std::fs::read(blob_file_path(&library.blob_dir, hash)).unwrap(),
            vec![0; 70000]
        );
        assert_eq!(
            library
                .reader()
                .unwrap()
                .query_row(
                    "SELECT length(bytes) FROM cl_blobs WHERE hash=?1",
                    [hash],
                    |row| row.get::<_, usize>(0),
                )
                .unwrap(),
            0
        );
        assert!(!library.migrate_blob_to_fs_step().unwrap());
    }
    library
        .writer()
        .unwrap()
        .execute(
            "UPDATE cl_blobs SET bytes=zeroblob(70000) WHERE hash='small'",
            [],
        )
        .unwrap();
    assert!(library.migrate_blob_to_fs_step().unwrap());
    assert!(!library.migrate_blob_to_fs_step().unwrap());
}

#[test]
fn membership_archives_once_and_returns_both_action_ids() {
    let library = Library::open().unwrap();
    library.set_local_device("test".into()).unwrap();
    library.mutate(r#"{"op":"save_list","list":{"id":"group","name":"group","kind":"manual"},"compact":true}"#).unwrap();
    let id = text(&library, "one atomic group action");
    let before = library.generation();
    library.membership("group", id, true).unwrap();
    assert_eq!(library.generation(), before + 1);
    let manual: Value = serde_json::from_str(
        &library
            .query(r#"{"op":"items","list_id":"group","include_total":false}"#)
            .unwrap(),
    )
    .unwrap();
    let archived = manual["items"][0]["id"].as_u64().unwrap();
    assert_ne!(archived, id);
    let aliases = manual["items"][0]["related_ids"].as_array().unwrap();
    assert!(aliases.contains(&json!(id)));
    assert!(aliases.contains(&json!(archived)));
    assert_eq!(
        library.snapshot(archived).unwrap().text.as_deref(),
        Some("one atomic group action")
    );
    library
        .mutate(&json!({"op":"delete_items","ids":[archived],"compact":true}).to_string())
        .unwrap();
    assert!(page(&library, "")["items"].as_array().unwrap().is_empty());
}

#[test]
fn normalized_payload_survives_legacy_backfill_and_metadata_edits() {
    let library = Library::open().unwrap();
    library.set_local_device("test".into()).unwrap();
    let id = text(&library, "complete native content");
    let bytes = encode(&library.snapshot(id).unwrap(), "local").unwrap();
    let hash = hex::encode(Sha256::digest(&bytes));
    {
        let db = library.writer().unwrap();
        // Restore the actual old representation, including an incomplete
        // projection. A crash/restart can resume from the persisted cursor.
        db.execute(
            "UPDATE cl_items SET payload=?1,blob_hash=NULL WHERE id=?3",
            params![bytes, hash, id],
        )
        .unwrap();
        db.execute("DELETE FROM cl_blobs WHERE hash=?1", [&hash])
            .unwrap();
        db.execute("DELETE FROM cl_item_metadata", []).unwrap();
    }
    assert_eq!(
        library.snapshot(id).unwrap().text.as_deref(),
        Some("complete native content")
    );
    finish_metadata(&library);
    library.migrate_payload_step().unwrap();
    library
        .mutate(
            &json!({"op":"set_item_title","id":id,"title":"changed","compact":true}).to_string(),
        )
        .unwrap();
    assert_eq!(
        library.snapshot(id).unwrap().text.as_deref(),
        Some("complete native content")
    );
    let db = library.reader().unwrap();
    assert!(
        db.query_row(
            "SELECT payload IS NULL FROM cl_items WHERE id=?1",
            [id],
            |r| r.get::<_, bool>(0)
        )
        .unwrap()
    );
    // migrate_payload_step 现在把 bytes 写到文件系统，cl_blobs.bytes = x''；
    // 通过 load_blob_db 仍可读回完整内容。
    assert_eq!(
        load_blob_db(&db, &library.blob_dir, &hash)
            .unwrap()
            .as_deref(),
        Some(bytes.as_slice())
    );
}

#[test]
fn short_unicode_search_is_exact_during_backfill_and_after_edits() {
    let library = Library::open().unwrap();
    library.set_local_device("test".into()).unwrap();
    let id = text(&library, &format!("{}A测试İ literal%_", "x".repeat(510)));
    finish_metadata(&library);
    for needle in ["测", "测试", "%_", "i\u{307}", "A测"] {
        assert_eq!(
            page(&library, needle)["items"].as_array().unwrap().len(),
            1,
            "pending {needle}"
        );
    }
    while library.storage_step().unwrap() {}
    for needle in ["测", "测试", "%_", "i\u{307}", "a测"] {
        assert_eq!(
            page(&library, needle)["items"][0]["id"],
            id,
            "indexed {needle}"
        );
    }
    assert!(
        page(&library, "不存在")["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    library
        .mutate(&json!({"op":"set_item_title","id":id,"title":"分组","compact":true}).to_string())
        .unwrap();
    assert_eq!(page(&library, "分组")["items"][0]["id"], id);
    while library.storage_step().unwrap() {}
    library
        .mutate(&json!({"op":"set_item_title","id":id,"title":"","compact":true}).to_string())
        .unwrap();
    assert!(
        page(&library, "分组")["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(page(&library, "测试")["total_exact"], false);
}

#[test]
fn expiry_hides_rows_before_any_cleanup_and_usage_does_not_recount() {
    let library = Library::open().unwrap();
    let id = text(&library, "expired");
    let before = library.counts_generation.load(Ordering::Acquire);
    library.mark_used(id).unwrap();
    assert_eq!(library.counts_generation.load(Ordering::Acquire), before);
    library
        .writer()
        .unwrap()
        .execute("UPDATE cl_items SET copied=1,imported=1 WHERE id=?1", [id])
        .unwrap();
    assert!(page(&library, "")["items"].as_array().unwrap().is_empty());
    assert!(library.snapshot(id).is_err());
    assert!(library.prune_step().unwrap());
}

#[test]
fn smart_predicates_match_reference_for_missing_sources_unicode_and_dates() {
    let library = Library::open().unwrap();
    let id = text(&library, "数据库 test");
    library
        .writer()
        .unwrap()
        .execute(
            "UPDATE cl_items SET title='Résumé',ocr_text='OCR文字' WHERE id=?1",
            [id],
        )
        .unwrap();
    let db = library.reader().unwrap();
    let raw: String = db
        .query_row("SELECT summary FROM cl_items WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .unwrap();
    let s: Summary = serde_json::from_str(&raw).unwrap();
    let q = json!({"day_start_ms":0,"day_end_ms":u64::MAX,"rule_windows":{"0":{"start":0,"end":u64::MAX}}});
    for (field, op, value) in [
        ("app", "is_not", "unknown"),
        ("device", "is", "unknown"),
        ("kind", "is", "text"),
        ("text", "contains", "数据库"),
        ("text", "contains", "RÉSUMÉ"),
        ("text", "not_contains", "missing"),
        ("date", "today", ""),
    ] {
        let r = Rule {
            field: field.into(),
            op: op.into(),
            value: value.into(),
            value2: String::new(),
        };
        let mut params = vec![];
        let predicate = query::rule(0, &r, &q, &mut params);
        let actual: bool = db
            .query_row(
                &format!("SELECT COALESCE({predicate},0) FROM cl_items i WHERE i.id={id}"),
                params_from_iter(params.iter()),
                |row| row.get(0),
            )
            .unwrap();
        let expected = matches_rule(0, &r, &s, "数据库 test", "résumé", "ocr文字", &q);
        assert_eq!(actual, expected, "{field}/{op}/{value}");
    }
}

#[test]
fn member_frontier_keeps_concurrent_removal_and_causal_readd_for_all_orders() {
    let make = |author: &str, deleted, clock: Value| json!({"op_id":author,"key":"member:list:archive:item","revision":1,"author":author,"clock":clock,"deleted":deleted,"value":{"list_id":"list","item_id":"archive:item"}});
    let ops = [
        make("a", false, json!({"a":1})),
        make("b", true, json!({"b":1})),
        make("c", false, json!({"a":1,"c":1})),
    ];
    for order in [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ] {
        let library = Library::open().unwrap();
        for index in order {
            library.merge_operations(vec![ops[index].clone()]).unwrap();
        }
        {
            let db = library.reader().unwrap();
            assert!(
                db.query_row(
                    "SELECT deleted FROM cl_records WHERE key='member:list:archive:item'",
                    [],
                    |r| r.get::<_, bool>(0)
                )
                .unwrap()
            );
            assert_eq!(
                db.query_row("SELECT COUNT(*) FROM cl_member_frontier", [], |r| r
                    .get::<_, u64>(0))
                    .unwrap(),
                2
            );
        }
        library.merge_operations(vec![json!({"op_id":"again","key":"member:list:archive:item","revision":2,"author":"a","clock":{"a":2,"b":1,"c":1},"deleted":false,"value":{"list_id":"list","item_id":"archive:item"}})]).unwrap();
        let db = library.reader().unwrap();
        assert!(
            !db.query_row(
                "SELECT deleted FROM cl_records WHERE key='member:list:archive:item'",
                [],
                |r| r.get::<_, bool>(0)
            )
            .unwrap()
        );
        assert_eq!(
            db.query_row("SELECT COUNT(*) FROM cl_member_frontier", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            1
        );
    }
}

#[test]
fn canceled_counts_stop_sql_and_do_not_block_an_interactive_page() {
    let library = Arc::new(Library::open().unwrap());
    let id = text(&library, "visible");
    finish_metadata(&library);
    {
        let db = library.writer().unwrap();
        db.execute_batch("WITH RECURSIVE n(x) AS (VALUES(2) UNION ALL SELECT x+1 FROM n WHERE x<10000) INSERT INTO cl_item_metadata(id,global_id,digest,summary,payload,search_text,search_index,copied,imported,ordinary,remote) SELECT x,'row:'||x,'digest:'||x,'{\"kind\":\"text\",\"preview\":\"test\"}',1,'test','test',9223372036854770000,0,1,0 FROM n;").unwrap();
    }
    let state: Value = serde_json::from_str(&library.state().unwrap()).unwrap();
    let requests = std::iter::once(json!({"list_id":null}))
        .chain(
            state["lists"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| json!({"list_id":v["id"]})),
        )
        .collect::<Vec<_>>();
    // Hold the readers until registration and cancellation have happened. A
    // scheduler pause after observing the key must not let SQL finish first.
    let readers = library
        .readers
        .iter()
        .map(|reader| reader.lock().unwrap())
        .collect::<Vec<_>>();
    let worker = library.clone();
    let thread = std::thread::spawn(move || {
        worker
            .query(&json!({"op":"counts","query_id":"cancel-test","requests":requests}).to_string())
    });
    while !library
        .query_cancellations
        .lock()
        .unwrap()
        .contains_key("cancel-test")
    {
        std::thread::yield_now();
    }
    library
        .query("{\"op\":\"cancel_query\",\"query_id\":\"cancel-test\"}")
        .unwrap();
    drop(readers);
    assert!(thread.join().unwrap().is_err());
    assert!(library.query_cancellations.lock().unwrap().is_empty());
    assert_eq!(page(&library, "visible")["items"][0]["id"], id);
}

/// Repeatable scale probe; separate from correctness tests and from real-device
/// acceptance. Bodies are shared, so this measures metadata cardinality only.
#[test]
#[ignore = "explicit scale benchmark"]
fn scale_benchmark() {
    let rows = std::env::var("DESKFUSE_CLIPBOARD_BENCH_ROWS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(100_000);
    let library = Library::open().unwrap();
    finish_metadata(&library);
    let now = now_ms();
    {
        let mut db = library.writer().unwrap();
        let tx = db.transaction().unwrap();
        tx.execute_batch(&format!("WITH RECURSIVE n(x) AS (VALUES(1) UNION ALL SELECT x+1 FROM n WHERE x<{rows}) INSERT INTO cl_item_metadata(id,global_id,digest,summary,payload,search_text,search_index,copied,imported,ordinary,remote) SELECT x,'row:'||x,'digest:'||x,json_object('id',x,'kind','text','mime','text/plain','length',20,'preview',CASE WHEN x%997=0 THEN '测试 benchmark' ELSE 'ordinary text' END,'source','local','file_count',0,'has_image',json('false'),'copied_at_ms',{now}),1,'ordinary text',CASE WHEN x%997=0 THEN '测试 benchmark' ELSE 'ordinary text' END,{now}+x,0,1,0 FROM n;
            INSERT INTO cl_items_fts(rowid,search_index) SELECT id,search_index FROM cl_item_metadata;
            INSERT INTO cl_short_terms SELECT '测试',id FROM cl_item_metadata WHERE id%997=0;")).unwrap();
        for i in 0..1000 {
            tx.execute(
                "INSERT INTO cl_records VALUES(?1,1,'bench',0,?2)",
                params![
                    format!("list:group:{i}"),
                    json!({"id":format!("group:{i}"),"name":"group","kind":"manual","position":i})
                        .to_string()
                ],
            )
            .unwrap();
        }
        tx.execute("INSERT INTO cl_item_metadata SELECT ?1,'archive:row:1',digest,summary,payload,search_text,search_index,copied,imported,0,blob_hash,origin_device,title,ocr_text,ocr_version,remote,last_used FROM cl_item_metadata WHERE id=1", [rows+1]).unwrap();
        tx.execute("INSERT INTO cl_records VALUES('member:group:999:archive:row:1',1,'bench',0,'{\"list_id\":\"group:999\",\"item_id\":\"archive:row:1\"}')", []).unwrap();
        tx.commit().unwrap();
    }
    for (label, query) in [
        ("page", json!({"include_total":false})),
        (
            "short-search",
            json!({"query":"测试","include_total":false}),
        ),
        (
            "fts-search",
            json!({"query":"benchmark","include_total":false}),
        ),
        (
            "sparse-group",
            json!({"list_id":"group:999","include_total":false}),
        ),
    ] {
        let mut times = Vec::new();
        let mut cursor = Value::Null;
        for _ in 0..20 {
            let mut request = query.clone();
            request["op"] = json!("items");
            if label == "page" {
                request["cursor"] = cursor.clone();
            }
            let start = std::time::Instant::now();
            let reply: Value =
                serde_json::from_str(&library.query(&request.to_string()).unwrap()).unwrap();
            times.push(start.elapsed().as_secs_f64() * 1000.0);
            cursor = reply["next_cursor"].clone();
            if label == "sparse-group" {
                assert_eq!(reply["items"].as_array().unwrap().len(), 1);
            }
        }
        times.sort_by(f64::total_cmp);
        eprintln!(
            "clipboard-scale rows={rows} groups=1000 operation={label} p50_ms={:.2} p95_ms={:.2}",
            times[10], times[18]
        );
        assert!(times[18] < 300.0, "{label} took {:.2} ms", times[18]);
    }
}

#[test]
#[ignore = "explicit read-only source database clone benchmark"]
fn database_clone_benchmark() {
    let source = std::env::var("DESKFUSE_CLIPBOARD_BENCH_DATABASE")
        .expect("supply a source SQLite database");
    let directory = tempfile::tempdir().unwrap();
    // SQLite backup includes committed WAL data. The original is read-only;
    // migrations, sorting and title edits only touch this temporary clone.
    let result = std::process::Command::new("python3").args(["-c", "import sqlite3,sys,pathlib; a=sqlite3.connect(pathlib.Path(sys.argv[1]).as_uri()+'?mode=ro',uri=True); b=sqlite3.connect(sys.argv[2]); a.backup(b); b.close(); a.close()", &source, directory.path().join("state.sqlite3").to_str().unwrap()]).status().unwrap();
    assert!(result.success());
    let started = std::time::Instant::now();
    let library = Library::open_database(directory).unwrap();
    eprintln!(
        "clipboard-clone startup_ms={:.2}",
        started.elapsed().as_secs_f64() * 1000.0
    );
    library
        .set_local_device("isolated-benchmark".into())
        .unwrap();
    let id: u64=library.reader().unwrap().query_row("SELECT id FROM cl_items WHERE ordinary=1 AND json_extract(summary,'$.kind')='text' ORDER BY id DESC LIMIT 1",[],|r|r.get(0)).unwrap();
    let started = std::time::Instant::now();
    library.snapshot(id).unwrap();
    eprintln!(
        "clipboard-clone initial_text_restore_ms={:.2}",
        started.elapsed().as_secs_f64() * 1000.0
    );
    finish_metadata(&library);
    library.mutate(r#"{"op":"save_list","list":{"id":"bench-group","name":"bench-group","kind":"manual"},"compact":true}"#).unwrap();
    for label in ["page", "text-restore", "usage", "rename", "membership"] {
        let mut times = Vec::new();
        for n in 0..20 {
            let started = std::time::Instant::now();
            match label {
                "page" => {
                    page(&library, "");
                }
                "text-restore" => {
                    library.snapshot(id).unwrap();
                }
                "usage" => library.mark_used(id).unwrap(),
                "rename" => {
                    library.mutate(&json!({"op":"set_item_title","id":id,"title":format!("benchmark-{n}"),"compact":true}).to_string()).unwrap();
                }
                "membership" => library.membership("bench-group", id, n % 2 == 0).unwrap(),
                _ => unreachable!(),
            }
            times.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        times.sort_by(f64::total_cmp);
        eprintln!(
            "clipboard-clone operation={label} p50_ms={:.2} p95_ms={:.2}",
            times[10], times[18]
        );
        assert!(times[18] < 300.0, "{label} took {:.2} ms", times[18]);
    }
    let ids: Vec<u64> = library
        .reader()
        .unwrap()
        .prepare("SELECT id FROM cl_items WHERE ordinary=1 ORDER BY id DESC LIMIT 20")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<std::result::Result<_, _>>()
        .unwrap();
    let mut times = Vec::new();
    for id in ids {
        let started = std::time::Instant::now();
        library
            .mutate(&json!({"op":"delete_items","ids":[id],"compact":true}).to_string())
            .unwrap();
        times.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    times.sort_by(f64::total_cmp);
    let p95 = times[(times.len() * 95 / 100).min(times.len() - 1)];
    eprintln!("clipboard-clone operation=delete p95_ms={p95:.2}");
    assert!(p95 < 300.0);
}

#[test]
fn clear_history_is_immediate_preserves_archives_and_allows_new_copies() {
    let library = Library::open().unwrap();
    library.set_local_device("test".into()).unwrap();
    library
        .mutate(
            &json!({"op":"save_list","list":{"id":"keep","name":"keep","kind":"manual"}})
                .to_string(),
        )
        .unwrap();
    let original = text(&library, "preserved archive");
    library.membership("keep", original, true).unwrap();
    let other = text(&library, "remove ordinary");
    let archive: Value = serde_json::from_str(
        &library
            .query("{\"op\":\"items\",\"list_id\":\"keep\"}")
            .unwrap(),
    )
    .unwrap();
    let archived = archive["items"][0]["id"].as_u64().unwrap();
    library.clear_history().unwrap();
    assert!(library.snapshot(original).is_err());
    assert!(library.snapshot(other).is_err());
    assert!(library.snapshot(archived).is_ok());
    assert_eq!(page(&library, "")["items"][0]["id"], archived);
    while library.prune_step().unwrap() {}
    let newer = text(&library, "new after clear");
    assert!(library.snapshot(newer).is_ok());
    assert_eq!(page(&library, "new after clear")["items"][0]["id"], newer);
}

#[test]
fn batched_group_changes_rollback_together_and_publish_one_revision() {
    let library = Library::open().unwrap();
    library.set_local_device("test".into()).unwrap();
    for (i, id) in ["first", "second"].iter().enumerate() {
        library
            .mutate(
                &json!({"op":"save_list","list":{"id":id,"name":id,"kind":"manual","position":i}})
                    .to_string(),
            )
            .unwrap();
    }
    let before = library.generation();
    library
        .mutate("{\"op\":\"reorder\",\"ids\":[\"second\",\"first\"],\"compact\":true}")
        .unwrap();
    assert_eq!(library.generation(), before + 1);
    {
        let db = library.writer().unwrap();
        db.execute_batch("CREATE TRIGGER reject_second BEFORE UPDATE ON cl_records WHEN new.key='list:second' BEGIN SELECT RAISE(FAIL,'injected write failure'); END;").unwrap();
    }
    assert!(
        library
            .mutate("{\"op\":\"reorder\",\"ids\":[\"first\",\"second\"],\"compact\":true}")
            .is_err()
    );
    let lists = lists_db(&library.reader().unwrap()).unwrap();
    assert_eq!(lists.iter().find(|l| l.id == "second").unwrap().position, 0);
    assert_eq!(lists.iter().find(|l| l.id == "first").unwrap().position, 1);
}

#[test]
fn background_publication_intent_failure_rolls_back_metadata() {
    let library = Arc::new(Library::open().unwrap());
    library.set_local_device("test".into()).unwrap();
    let id = text(&library, "durable update");
    // Run the actual maintenance queue, then reject only the intent insertion.
    work::start(&library);
    library.writer().unwrap().execute_batch("CREATE TRIGGER reject_intent BEFORE INSERT ON cl_pending_publications BEGIN SELECT RAISE(FAIL,'injected queue failure'); END;").unwrap();
    assert!(
        library
            .mutate(
                &json!({"op":"set_item_title","id":id,"title":"should rollback","compact":true})
                    .to_string()
            )
            .is_err()
    );
    let title: Option<String> = library
        .reader()
        .unwrap()
        .query_row("SELECT title FROM cl_items WHERE id=?1", [id], |r| r.get(0))
        .unwrap();
    assert!(title.is_none());
}

#[test]
fn set_item_text_rewrites_content_in_place() {
    let library = Library::open().unwrap();
    library.set_local_device("test".into()).unwrap();
    let id = text(&library, "原始文本内容");
    finish_metadata(&library);
    library
        .mutate(
            &json!({"op":"set_item_title","id":id,"title":"保留标题","compact":true}).to_string(),
        )
        .unwrap();
    let before = library.generation();
    library
        .mutate(
            &json!({"op":"set_item_text","id":id,"text":"编辑后的文本","compact":true}).to_string(),
        )
        .unwrap();
    assert!(library.generation() > before);
    // 内容原地替换：同一 id 读出新文本，粘贴表示不再携带旧文本。
    let snapshot = library.snapshot(id).unwrap();
    assert_eq!(snapshot.text.as_deref(), Some("编辑后的文本"));
    let types: Vec<&str> = snapshot
        .items
        .iter()
        .flat_map(|item| item.representations.iter().map(|r| r.type_name.as_str()))
        .collect();
    assert_eq!(types, ["public.utf8-plain-text"]);
    // 列表行 preview 更新，标题等其它字段保持。
    let items = page(&library, "")["items"].as_array().unwrap().clone();
    let row = items
        .iter()
        .find(|row| row["id"].as_u64() == Some(id))
        .unwrap();
    assert_eq!(row["preview"], "编辑后的文本");
    assert_eq!(row["title"], "保留标题");
    // 搜索命中新文本，不再命中旧文本。
    assert_eq!(page(&library, "编辑后")["items"][0]["id"], id);
    assert!(
        page(&library, "原始文本")["items"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    // digest 已更新：再次复制原文本会作为新条目入库，而不是合并进已编辑条目。
    let recopied = text(&library, "原始文本内容");
    assert_ne!(recopied, id);
}

#[test]
fn set_item_text_keeps_rtf_representation() {
    let library = Library::open().unwrap();
    library.set_local_device("test".into()).unwrap();
    let id = text(&library, "plain before rich edit");
    let rtf: Vec<u8> = b"{\\rtf1\\b rich}".to_vec();
    library
        .mutate(
            &json!({"op":"set_item_text","id":id,"text":"rich edit","rtf":rtf,"compact":true})
                .to_string(),
        )
        .unwrap();
    let snapshot = library.snapshot(id).unwrap();
    assert_eq!(snapshot.text.as_deref(), Some("rich edit"));
    let rtf_representation = snapshot
        .items
        .iter()
        .flat_map(|item| &item.representations)
        .find(|r| r.type_name == "public.rtf")
        .unwrap();
    assert_eq!(rtf_representation.data, rtf);
}

#[test]
fn set_item_text_updates_archived_copy_in_manual_list() {
    let library = Library::open().unwrap();
    library.set_local_device("test".into()).unwrap();
    library.mutate(r#"{"op":"save_list","list":{"id":"group","name":"group","kind":"manual"},"compact":true}"#).unwrap();
    let id = text(&library, "归档前的文本");
    library.membership("group", id, true).unwrap();
    let manual: Value = serde_json::from_str(
        &library
            .query(r#"{"op":"items","list_id":"group","include_total":false}"#)
            .unwrap(),
    )
    .unwrap();
    let archived = manual["items"][0]["id"].as_u64().unwrap();
    assert_ne!(archived, id);
    library
        .mutate(
            &json!({"op":"set_item_text","id":archived,"text":"归档后的文本","compact":true})
                .to_string(),
        )
        .unwrap();
    assert_eq!(
        library.snapshot(archived).unwrap().text.as_deref(),
        Some("归档后的文本")
    );
    let manual: Value = serde_json::from_str(
        &library
            .query(r#"{"op":"items","list_id":"group","include_total":false}"#)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(manual["items"][0]["preview"], "归档后的文本");
}

#[test]
fn set_item_text_rejects_blank_unsupported_and_remote_rows() {
    let library = Library::open().unwrap();
    library.set_local_device("test".into()).unwrap();
    let id = text(&library, "keep me");
    assert!(
        library
            .mutate(&json!({"op":"set_item_text","id":id,"text":"   ","compact":true}).to_string())
            .is_err()
    );
    let image = library
        .ingest(
            &PasteboardSnapshot {
                png: Some(vec![137, 80, 78, 71]),
                ..Default::default()
            },
            "local",
        )
        .unwrap()
        .unwrap();
    assert!(
        library
            .mutate(
                &json!({"op":"set_item_text","id":image,"text":"new","compact":true}).to_string()
            )
            .is_err()
    );
    // 远端索引行：编辑会被下一次 reconcile 覆盖，直接拒绝。
    library
        .merge_operations(vec![json!({
            "op_id": "op-remote-text",
            "key": "index:remote-text-row",
            "revision": 1,
            "author": "peer-device",
            "clock": {"peer-device": 1},
            "deleted": false,
            "value": {
                "id": "remote-text-row",
                "digest": "text:remote-text-row",
                "summary": {
                    "id": 0, "copied_at_ms": now_ms(), "mime": "text/plain",
                    "length": 11, "preview": "remote text", "source": "远端 peer-device",
                    "has_image": false, "file_count": 0, "source_device_id": "peer-device",
                    "source_app_id": null, "source_app_name": null, "kind": "text"
                },
                "title": null,
                "ocr_text": null,
                "search_text": "remote text",
                "blob_hash": "d".repeat(64),
            },
        })])
        .unwrap();
    let remote_id: u64 = library
        .reader()
        .unwrap()
        .query_row(
            "SELECT id FROM cl_items WHERE global_id='remote-text-row'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        library
            .mutate(
                &json!({"op":"set_item_text","id":remote_id,"text":"new","compact":true})
                    .to_string()
            )
            .is_err()
    );
    assert_eq!(
        library.snapshot(id).unwrap().text.as_deref(),
        Some("keep me")
    );
}

/// 与任何归档都没有身份关系的同文本历史行，复刻导入历史与跨设备同步造成的重复。
fn duplicate_row(library: &Library, value: &str, remote: bool) -> u64 {
    let db = library.writer().unwrap();
    let mut summary = json!({
        "kind": "text",
        "mime": "text/plain",
        "length": value.len(),
        "preview": value,
        "source": "本机",
        "has_image": false,
        "file_count": 0,
    });
    db.execute(
        "INSERT INTO cl_items(global_id,digest,summary,imported,search_text,ordinary,remote) VALUES(?1,?2,?3,?4,?5,1,?6)",
        params![
            uuid::Uuid::new_v4().to_string(),
            format!("legacy:{}", uuid::Uuid::new_v4()),
            serde_json::to_string(&summary).unwrap(),
            now_ms(),
            value.to_lowercase(),
            remote as i64
        ],
    )
    .unwrap();
    let id = db.last_insert_rowid() as u64;
    // summary 必须回填自身 id，读取侧的 Summary 结构体要求该字段。
    summary["id"] = json!(id);
    db.execute(
        "UPDATE cl_items SET summary=?1 WHERE id=?2",
        params![serde_json::to_string(&summary).unwrap(), id],
    )
    .unwrap();
    id
}

/// 冻结归档行加手动列表成员关系，复刻从别的设备或导入历史带进来的已命名条目。
fn named_archive(library: &Library, list: &str, value: &str, title: &str) -> u64 {
    let global_id = format!("archive:{}", uuid::Uuid::new_v4());
    let mut summary = json!({
        "kind": "text",
        "mime": "text/plain",
        "length": value.len(),
        "preview": value,
        "source": "本机",
        "has_image": false,
        "file_count": 0,
    });
    let db = library.writer().unwrap();
    db.execute(
        "INSERT INTO cl_items(global_id,digest,summary,imported,search_text,ordinary,title) VALUES(?1,?2,?3,?4,?5,0,?6)",
        params![
            global_id,
            format!("legacy:{}", uuid::Uuid::new_v4()),
            serde_json::to_string(&summary).unwrap(),
            now_ms(),
            value.to_lowercase(),
            title
        ],
    )
    .unwrap();
    let id = db.last_insert_rowid() as u64;
    summary["id"] = json!(id);
    db.execute(
        "UPDATE cl_items SET summary=?1 WHERE id=?2",
        params![serde_json::to_string(&summary).unwrap(), id],
    )
    .unwrap();
    db.execute(
        "INSERT INTO cl_records(key,revision,author,deleted,value) VALUES(?1,1,'test',0,?2)",
        params![
            format!("member:{list}:{global_id}"),
            json!({"list_id": list, "item_id": global_id}).to_string()
        ],
    )
    .unwrap();
    id
}

fn text_rows(library: &Library, value: &str) -> i64 {
    library
        .reader()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM cl_items WHERE search_text=?1",
            [value.to_lowercase()],
            |r| r.get(0),
        )
        .unwrap()
}

fn used_at(library: &Library, id: u64) -> u64 {
    library
        .reader()
        .unwrap()
        .query_row(
            "SELECT COALESCE(last_used,0) FROM cl_items WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .unwrap()
}

fn snapshot_of(value: &str) -> PasteboardSnapshot {
    PasteboardSnapshot {
        text: Some(value.into()),
        ..Default::default()
    }
}

#[test]
fn repeating_text_reuses_the_named_archive_without_a_new_row() {
    let library = Library::open().unwrap();
    library.set_local_device("test".into()).unwrap();
    library
        .mutate(r#"{"op":"save_list","list":{"id":"group","name":"group","kind":"manual"},"compact":true}"#)
        .unwrap();
    let base = text(&library, "90731230");
    library.membership("group", base, true).unwrap();
    let manual: Value = serde_json::from_str(
        &library
            .query(r#"{"op":"items","list_id":"group","include_total":false}"#)
            .unwrap(),
    )
    .unwrap();
    let archived = manual["items"][0]["id"].as_u64().unwrap();
    library
        .mutate(
            &json!({"op":"set_item_title","id":archived,"title":"yz定制自动化测试店铺","compact":true})
                .to_string(),
        )
        .unwrap();
    let before = text_rows(&library, "90731230");

    let reused = library
        .ingest(&snapshot_of("90731230"), "local")
        .unwrap()
        .unwrap();
    assert_eq!(reused, base);
    assert_eq!(text_rows(&library, "90731230"), before);
    assert!(used_at(&library, archived) > 0);

    // 全局历史展示归档的普通来源行，手动列表展示归档本身，标题与内容都保留。
    let items = page(&library, "");
    let items = items["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"].as_u64().unwrap(), base);
    let manual: Value = serde_json::from_str(
        &library
            .query(r#"{"op":"items","list_id":"group","include_total":false}"#)
            .unwrap(),
    )
    .unwrap();
    assert_eq!(manual["items"].as_array().unwrap().len(), 1);
    assert_eq!(manual["items"][0]["id"].as_u64().unwrap(), archived);
    assert_eq!(
        manual["items"][0]["title"].as_str().unwrap(),
        "yz定制自动化测试店铺"
    );
    assert_eq!(
        library.snapshot(archived).unwrap().text.as_deref(),
        Some("90731230")
    );
}

#[test]
fn historical_duplicates_show_only_the_named_archive() {
    let library = Library::open().unwrap();
    library.set_local_device("test".into()).unwrap();
    library
        .mutate(r#"{"op":"save_list","list":{"id":"group","name":"group","kind":"manual"},"compact":true}"#)
        .unwrap();
    let archived = named_archive(&library, "group", "90731230", "yz定制自动化测试店铺");
    duplicate_row(&library, "90731230", false);
    duplicate_row(&library, "90731230", true);

    let items = page(&library, "");
    let items = items["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"].as_u64().unwrap(), archived);
    assert_eq!(items[0]["title"].as_str().unwrap(), "yz定制自动化测试店铺");

    let before = text_rows(&library, "90731230");
    let reused = library
        .ingest(&snapshot_of("90731230"), "local")
        .unwrap()
        .unwrap();
    assert_eq!(reused, archived);
    assert_eq!(text_rows(&library, "90731230"), before);
    assert!(used_at(&library, archived) > 0);
    let items = page(&library, "");
    let items = items["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"].as_u64().unwrap(), archived);
}

#[test]
fn an_untitled_archive_does_not_absorb_repeated_text() {
    let library = Library::open().unwrap();
    library.set_local_device("test".into()).unwrap();
    library
        .mutate(r#"{"op":"save_list","list":{"id":"group","name":"group","kind":"manual"},"compact":true}"#)
        .unwrap();
    named_archive(&library, "group", "90731230", "");
    duplicate_row(&library, "90731230", false);

    let before = text_rows(&library, "90731230");
    library
        .ingest(&snapshot_of("90731230"), "local")
        .unwrap()
        .unwrap();
    // 没有标题就没有关联依据：仍按原有 digest/文本规则复用，重复行保持可见。
    assert_eq!(text_rows(&library, "90731230"), before);
    let items = page(&library, "");
    assert_eq!(items["items"].as_array().unwrap().len(), 2);
}

#[test]
fn optional_count_deadline_interrupts_sqlite_and_reader_remains_usable() {
    let db = Connection::open_in_memory().unwrap();
    let canceled = Arc::new(std::sync::atomic::AtomicBool::new(false));
    install_query_progress_handler(&db, canceled.clone(), Some(std::time::Instant::now()));
    let scan = "WITH RECURSIVE numbers(n) AS (VALUES(1) UNION ALL SELECT n+1 FROM numbers WHERE n<10000) SELECT sum(n) FROM numbers";
    let error = db
        .query_row(scan, [], |row| row.get::<_, i64>(0))
        .unwrap_err();
    assert!(
        matches!(error, rusqlite::Error::SqliteFailure(code, _) if code.code == rusqlite::ErrorCode::OperationInterrupted)
    );
    install_query_progress_handler(&db, canceled, None);
    assert_eq!(
        db.query_row(scan, [], |row| row.get::<_, i64>(0)).unwrap(),
        50_005_000
    );
    db.progress_handler(0, None::<fn() -> bool>);
}

fn item_title(library: &Library, id: u64) -> Option<String> {
    library
        .reader()
        .unwrap()
        .query_row("SELECT title FROM cl_items WHERE id=?1", [id], |r| r.get(0))
        .unwrap()
}

fn row_id(library: &Library, global_id: &str) -> u64 {
    library
        .reader()
        .unwrap()
        .query_row(
            "SELECT id FROM cl_items WHERE global_id=?1",
            [global_id],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn archived_item_title_syncs_to_peer_and_survives_text_edit() {
    let library = Library::open().unwrap();
    library.set_local_device("a".into()).unwrap();
    let peer = Library::open().unwrap();
    peer.set_local_device("b".into()).unwrap();
    library.mutate(r#"{"op":"save_list","list":{"id":"group","name":"group","kind":"manual"},"compact":true}"#).unwrap();
    let id = text(&library, "90731230");
    library.membership("group", id, true).unwrap();
    let manual: Value = serde_json::from_str(
        &library
            .query(r#"{"op":"items","list_id":"group","include_total":false}"#)
            .unwrap(),
    )
    .unwrap();
    let archived = manual["items"][0]["id"].as_u64().unwrap();
    let gid: String = library
        .reader()
        .unwrap()
        .query_row(
            "SELECT global_id FROM cl_items WHERE id=?1",
            [archived],
            |r| r.get(0),
        )
        .unwrap();
    library
        .mutate(
            &json!({"op":"set_item_title","id":archived,"title":"测试店铺","compact":true})
                .to_string(),
        )
        .unwrap();
    assert_eq!(item_title(&library, archived).as_deref(), Some("测试店铺"));

    peer.merge_operations(library.export_operations(0, 100).unwrap())
        .unwrap();
    let remote_archive = row_id(&peer, &gid);
    assert_eq!(
        item_title(&peer, remote_archive).as_deref(),
        Some("测试店铺")
    );

    // 对端改名后回传；编辑归档正文不丢标题。
    // 按 seq 游标增量拉取：日志压缩会删掉被支配的旧操作，条数不再单调。
    let after = peer
        .export_operations(0, 100)
        .unwrap()
        .last()
        .and_then(|operation| operation["seq"].as_u64())
        .unwrap();
    peer.mutate(
        &json!({"op":"set_item_title","id":remote_archive,"title":"连锁","compact":true})
            .to_string(),
    )
    .unwrap();
    library
        .merge_operations(peer.export_operations(after, 100).unwrap())
        .unwrap();
    assert_eq!(item_title(&library, archived).as_deref(), Some("连锁"));
    library
        .mutate(
            &json!({"op":"set_item_text","id":archived,"text":"90731231","compact":true})
                .to_string(),
        )
        .unwrap();
    assert_eq!(item_title(&library, archived).as_deref(), Some("连锁"));
    let fresh = Library::open().unwrap();
    fresh.set_local_device("c".into()).unwrap();
    fresh
        .merge_operations(library.export_operations(0, 100).unwrap())
        .unwrap();
    assert_eq!(
        item_title(&fresh, row_id(&fresh, &gid)).as_deref(),
        Some("连锁")
    );

    // 清空标题同样同步。
    library
        .mutate(&json!({"op":"set_item_title","id":archived,"title":"","compact":true}).to_string())
        .unwrap();
    peer.merge_operations(library.export_operations(0, 200).unwrap())
        .unwrap();
    assert_eq!(item_title(&peer, remote_archive), None);
}

#[test]
fn history_title_edited_on_peer_updates_origin_row() {
    let library = Library::open().unwrap();
    library.set_local_device("a".into()).unwrap();
    let peer = Library::open().unwrap();
    peer.set_local_device("b".into()).unwrap();
    let id = text(&library, "cloud_page_show_shop_nav");
    library
        .mutate(
            &json!({"op":"set_item_title","id":id,"title":"导航开关","compact":true}).to_string(),
        )
        .unwrap();
    peer.merge_operations(library.export_operations(0, 100).unwrap())
        .unwrap();
    let gid: String = library
        .reader()
        .unwrap()
        .query_row("SELECT global_id FROM cl_items WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .unwrap();
    let remote = row_id(&peer, &gid);
    assert_eq!(item_title(&peer, remote).as_deref(), Some("导航开关"));

    peer.mutate(
        &json!({"op":"set_item_title","id":remote,"title":"店铺导航","compact":true}).to_string(),
    )
    .unwrap();
    library
        .merge_operations(peer.export_operations(0, 100).unwrap())
        .unwrap();
    assert_eq!(item_title(&library, id).as_deref(), Some("店铺导航"));

    // 启动时批量 reconcile 不改写本机标题。
    library
        .writer()
        .unwrap()
        .execute("UPDATE cl_items SET title='本机' WHERE id=?1", [id])
        .unwrap();
    library.reconcile_index().unwrap();
    assert_eq!(item_title(&library, id).as_deref(), Some("本机"));
}

fn operations_for(library: &Library, key: &str) -> u64 {
    library
        .reader()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM cl_operations WHERE json_extract(body,'$.key')=?1",
            [key],
            |r| r.get(0),
        )
        .unwrap()
}

#[test]
fn operation_log_keeps_only_register_winner_and_ignores_replayed_losers() {
    let library = Library::open().unwrap();
    library.set_local_device("a".into()).unwrap();
    let id = text(&library, "压缩日志");
    library
        .mutate(&json!({"op":"set_item_title","id":id,"title":"旧标题","compact":true}).to_string())
        .unwrap();
    let stale = library.export_operations(0, 100).unwrap();
    let key = stale.last().unwrap()["key"].as_str().unwrap().to_string();
    library
        .mutate(&json!({"op":"set_item_title","id":id,"title":"新标题","compact":true}).to_string())
        .unwrap();
    assert_eq!(operations_for(&library, &key), 1);

    // 旧版本对端每次连接都会从头重放；落败操作不能重新进入日志。
    library.merge_operations(stale.clone()).unwrap();
    assert_eq!(operations_for(&library, &key), 1);
    assert_eq!(item_title(&library, id).as_deref(), Some("新标题"));

    // 新设备只拿到压缩后的日志，也收敛到相同状态。
    let fresh = Library::open().unwrap();
    fresh.set_local_device("b".into()).unwrap();
    fresh
        .merge_operations(library.export_operations(0, 100).unwrap())
        .unwrap();
    let gid: String = library
        .reader()
        .unwrap()
        .query_row("SELECT global_id FROM cl_items WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        item_title(&fresh, row_id(&fresh, &gid)).as_deref(),
        Some("新标题")
    );
}

#[test]
fn operation_log_keeps_member_frontier_only() {
    let key = "member:list:archive:item";
    let make = |op_id: &str, author: &str, revision: u64, deleted, clock: Value| json!({"op_id":op_id,"key":key,"revision":revision,"author":author,"clock":clock,"deleted":deleted,"value":{"list_id":"list","item_id":"archive:item"}});
    let library = Library::open().unwrap();
    library
        .merge_operations(vec![
            make("a", "a", 1, false, json!({"a":1})),
            make("b", "b", 1, true, json!({"b":1})),
            make("c", "c", 1, false, json!({"a":1,"c":1})),
        ])
        .unwrap();
    // a 被 c 支配；b 与 c 并发，二者都在前沿。
    assert_eq!(operations_for(&library, key), 2);
    library
        .merge_operations(vec![make("d", "a", 2, false, json!({"a":2,"b":1,"c":1}))])
        .unwrap();
    assert_eq!(operations_for(&library, key), 1);
}

#[test]
fn legacy_operation_log_is_compacted_once_without_losing_records() {
    let library = Library::open().unwrap();
    library.set_local_device("a".into()).unwrap();
    let id = text(&library, "旧日志");
    library
        .mutate(&json!({"op":"set_item_title","id":id,"title":"标题","compact":true}).to_string())
        .unwrap();
    let winner = library.export_operations(0, 100).unwrap().pop().unwrap();
    let key = winner["key"].as_str().unwrap().to_string();
    let orphan = "index:orphan";
    {
        let db = library.writer().unwrap();
        // 模拟旧版本：落败操作留在日志里，且有记录的胜者操作已缺失。
        let mut loser = winner.clone();
        loser.as_object_mut().unwrap().remove("seq");
        loser["op_id"] = json!("legacy-loser");
        loser["revision"] = json!(0);
        db.execute(
            "INSERT INTO cl_operations(op_id,body) VALUES('legacy-loser',?1)",
            [loser.to_string()],
        )
        .unwrap();
        let mut stray = loser.clone();
        stray["op_id"] = json!("legacy-orphan");
        stray["key"] = json!(orphan);
        db.execute(
            "INSERT INTO cl_operations(op_id,body) VALUES('legacy-orphan',?1)",
            [stray.to_string()],
        )
        .unwrap();
        db.execute("INSERT INTO cl_records VALUES(?1,9,'z',0,'{}')", [orphan])
            .unwrap();
        db.execute(
            "DELETE FROM cl_meta WHERE key=?1",
            ["operations_compacted_v1"],
        )
        .unwrap();
    }
    assert_eq!(operations_for(&library, &key), 2);
    let mut cursor = 0;
    let mut steps = 0;
    loop {
        let (next, more) = library.compact_operations_step(cursor).unwrap();
        cursor = next;
        steps += 1;
        if !more {
            break;
        }
        assert!(steps < 100);
    }
    assert_eq!(operations_for(&library, &key), 1);
    // 胜者缺失时保留现有操作，记录不能从同步日志里消失。
    assert_eq!(operations_for(&library, orphan), 1);
    let done: bool = library
        .reader()
        .unwrap()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM cl_meta WHERE key=?1)",
            ["operations_compacted_v1"],
            |r| r.get(0),
        )
        .unwrap();
    assert!(done);
}

#[test]
fn unused_record_clocks_table_is_dropped() {
    let library = Library::open().unwrap();
    let exists: bool = library
        .reader()
        .unwrap()
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='cl_clocks')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!exists);
}

#[test]
fn operation_compaction_uses_indexes_instead_of_nested_scans() {
    let library = Library::open().unwrap();
    let db = library.reader().unwrap();
    let mut statement = db
        .prepare(&format!(
            "EXPLAIN QUERY PLAN {}",
            storage::COMPACT_OPERATIONS_SQL
        ))
        .unwrap();
    let plan = statement
        .query_map(params![0, 1], |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        plan.iter().all(|step| !step.starts_with("SCAN")),
        "{plan:?}"
    );
}
