import XCTest
@testable import DeskFuse

@MainActor final class ClipboardPresentationTests: XCTestCase {
    private func item(_ id: UInt64, members: [String] = []) throws -> ClipboardLibraryItem {
        let data = try JSONSerialization.data(withJSONObject: ["id":id,"mime":"text/plain","length":4,"preview":"text","source":"local","has_image":false,"file_count":0,"memberships":members])
        return try JSONDecoder().decode(ClipboardLibraryItem.self, from: data)
    }

    func testFailureRollsBackOnlyItsOwnMutation() throws {
        let store = ClipboardLibraryStore()
        let page = try [item(1),item(2)]
        let deletion = store.beginMutation(["op":"delete_items","ids":[UInt64(1)]])
        _ = store.beginMutation(["op":"set_item_title","id":UInt64(2),"title":"new"])
        XCTAssertEqual(store.project(page, revision: 1).map(\.id), [2])
        store.rollbackMutation(deletion)
        let restored = store.project(page, revision: 1)
        XCTAssertEqual(restored.map(\.id), [1,2])
        XCTAssertEqual(restored.last?.title, "new")
        XCTAssertFalse(store.isLocallyDeleted(1))
    }

    func testReceiptProtectsOldPageUntilAuthoritativeRevisionArrives() throws {
        let store = ClipboardLibraryStore()
        let page = try [item(1)]
        let rename = store.beginMutation(["op":"set_item_title","id":UInt64(1),"title":"new"])
        try store.acknowledgeMutation(rename, response: "{\"revision\":8}")
        XCTAssertEqual(store.project(page, revision: 7).first?.title,"new")
        var authoritative = page; authoritative[0].title = "server"
        XCTAssertEqual(store.project(authoritative, revision: 8).first?.title,"server")
    }

    func testRemovingMembershipImmediatelyHidesRowInThatListAndRollsBack() throws {
        let store = ClipboardLibraryStore()
        let page = try [item(1,members:["first","second"])]
        let removal = store.beginMutation(["op":"membership","history_id":UInt64(1),"list_id":"first","present":false])
        XCTAssertTrue(store.project(page,revision:1,listID:"first").isEmpty)
        XCTAssertEqual(store.project(page,revision:1,listID:"second").first?.memberships,["second"])
        store.rollbackMutation(removal)
        XCTAssertEqual(store.project(page,revision:1,listID:"first").count,1)
    }

    func testDeletePublishesBeforeAnyAsyncTaskRunsAndHidesArchiveAliases() throws {
        let store = ClipboardLibraryStore()
        var archived = try item(9, members: ["group"])
        archived.related_ids = [1, 9]
        store.remember(archived)
        store.applyCounts(["all": 2, "group": 1], revision: 1)
        store.delete([9])
        // The callback has not yielded to the native writer, even with an
        // unavailable engine. Feedback belongs to this synchronous action.
        XCTAssertTrue(store.project(try [item(1), archived], revision: 1).isEmpty)
        XCTAssertTrue(store.isLocallyDeleted(1))
        XCTAssertEqual(store.counts, ["all": 1, "group": 0])
    }

    func testGroupCountAndUncachedTargetPageUpdateBeforeCommit() throws {
        let store = ClipboardLibraryStore()
        store.remember(try item(1))
        store.applyCounts(["all": 1, "group": 0], revision: 1)
        let addition = store.beginMutation(["op":"membership","history_id":UInt64(1),"list_id":"group","present":true])
        XCTAssertEqual(store.counts["group"], 1)
        XCTAssertEqual(store.project([],revision:nil,listID:"group").first?.id, 1)
        XCTAssertTrue(store.project([],revision:nil,listID:"group",search:"unmatched").isEmpty)
        try store.acknowledgeMutation(addition, response:"{\"revision\":8,\"counts_revision\":4}")
        store.applyCounts(["all":1,"group":0], revision:3)
        XCTAssertEqual(store.counts["group"], 1)
        store.applyCounts(["all":1,"group":1], revision:4)
        XCTAssertEqual(store.counts["group"], 1, "Do not apply a committed delta twice")
    }

    func testRapidMembershipTogglesAndFailureRollbackKeepCountsConsistent() throws {
        let store = ClipboardLibraryStore()
        store.remember(try item(1))
        store.applyCounts(["all":1,"group":0], revision:1)
        _ = store.beginMutation(["op":"membership","history_id":UInt64(1),"list_id":"group","present":true])
        let removal = store.beginMutation(["op":"membership","history_id":UInt64(1),"list_id":"group","present":false])
        XCTAssertEqual(store.counts["group"], 0)
        XCTAssertTrue(store.project([],revision:nil,listID:"group").isEmpty)
        store.rollbackMutation(removal)
        XCTAssertEqual(store.counts["group"], 1)
        XCTAssertEqual(store.project([],revision:nil,listID:"group").count, 1)
    }

    func testFailedRemovalDoesNotMakeFollowingAddCountTwice() throws {
        let store = ClipboardLibraryStore()
        store.remember(try item(1,members:["group"]))
        store.applyCounts(["all":1,"group":1], revision:1)
        let removal = store.beginMutation(["op":"membership","history_id":UInt64(1),"list_id":"group","present":false])
        _ = store.beginMutation(["op":"membership","history_id":UInt64(1),"list_id":"group","present":true])
        store.rollbackMutation(removal)
        XCTAssertEqual(store.counts["group"], 1)
    }

    func testNewAuthoritativeMembershipOverridesEarlierCountReceipts() throws {
        let store = ClipboardLibraryStore()
        store.remember(try item(1), revision:1)
        store.applyCounts(["all":1,"group":0], revision:1)
        let first = store.beginMutation(["op":"membership","history_id":UInt64(1),"list_id":"group","present":true])
        try store.acknowledgeMutation(first, response:"{\"revision\":2,\"counts_revision\":2}")
        store.applyCounts(["all":1,"group":1], revision:2)
        // A peer removes the item. The old local receipt remains bounded in
        // memory but must not suppress feedback for a new add.
        store.remember(try item(1), revision:4)
        store.applyCounts(["all":1,"group":0], revision:4)
        _ = store.beginMutation(["op":"membership","history_id":UInt64(1),"list_id":"group","present":true])
        XCTAssertEqual(store.counts["group"], 1)
    }
}
