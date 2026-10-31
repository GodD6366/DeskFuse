import Foundation
import AppKit
import ServiceManagement
import AVFoundation
import DeskFuseEngine

/// The engine surface the bridge may use. `EngineClient` is the only production
/// conformer, so the bridge shares the app's single engine and its query actor.
protocol MyGoLibraryQuerying: Sendable {
    func libraryQuery(_ request: String) async throws -> String
    func libraryMutate(_ request: String) async throws -> String
    func content(_ id: UInt64) async throws -> ClipboardContent
    func thumbnail(_ id: UInt64, maxPixelSize: Int) async throws -> Data
}

extension MyGoLibraryQuerying {
    func libraryMutate(_ request: String) async throws -> String { throw MyGoBridgeError(code: "unsupported_action", message: "library.mutate") }
    func content(_ id: UInt64) async throws -> ClipboardContent { throw MyGoBridgeError(code: "unsupported_action", message: "clipboard.content") }
    func thumbnail(_ id: UInt64, maxPixelSize: Int) async throws -> Data { throw MyGoBridgeError(code: "unsupported_action", message: "clipboard.thumbnail") }
}

extension EngineClient: MyGoLibraryQuerying {}

enum MyGoSnapshot {
    private static func nullable<T>(_ value: T?) -> Any { value.map { $0 as Any } ?? NSNull() }
    private static func name(_ path: InputPath) -> String {
        switch path { case .auto: "auto"; case .thunderbolt: "thunderbolt"; case .ethernet: "ethernet"; case .wiFi: "wifi" }
    }
    @MainActor private static var remoteApplicationRestriction: String {
        if #available(macOS 14.2, *) { return RemoteApplicationManager.viewerRestriction }
        return ""
    }

    /// `hw.model` 是原生本机产品图（Mac mini / MacBook 等）的判据。
    private static let modelIdentifier: String = {
        var size = 0
        guard sysctlbyname("hw.model", nil, &size, nil, 0) == 0, size > 0 else { return "" }
        var buffer = [CChar](repeating: 0, count: size)
        guard sysctlbyname("hw.model", &buffer, &size, nil, 0) == 0 else { return "" }
        return String(cString: buffer)
    }()

    @MainActor static func encode(_ store: AppStore) throws -> Data {
        let s = store.snapshot
        let selectedPage: String = switch store.selectedPage {
        case .devices: "devices"; case .clipboard: "clipboard"; case .input: "input"; case .files: "files"
        case .mouse: "mouse"; case .microphone: "microphone"; case .remoteApplications: "remote_applications"; case .settings: "settings"
        }
        var out: [String: Any] = [
            "schema": 1, "engine_ready": store.client != nil, "revision": String(s?.revision ?? 0),
            "app": ["version_label": SupportCenter.versionLabel, "starting": store.starting,
                    "loading": store.isEngineLoading, "error": nullable(store.error), "selected_page": selectedPage,
                    "settings_category": store.settingsDestination.category.rawValue, "keep_awake": store.keepAwake,
                    "hide_dock_icon": store.hideDockIcon, "workflows_enabled": store.workflowsEnabled,
                    "channel": UpdateInstallationChannel.current.rawValue]
        ]
        guard let s else { return try JSONSerialization.data(withJSONObject: out, options: [.sortedKeys]) }
        out["local"] = ["id": s.localId, "name": s.localName, "runtime_status": s.runtimeStatus,
                        "network_id": nullable(s.networkId), "error": nullable(s.error), "debug_logging": s.debugLoggingEnabled,
                        // 原生按 hw.model 选本机产品图；WebView 用同一个标识保持图标一致。
                        "model_identifier": Self.modelIdentifier]
        out["devices"] = s.devices.map { device -> [String: Any] in
            ["id": device.id, "name": device.name, "detail": device.detail, "endpoint": device.endpoint,
             "trusted": device.trusted, "connected": device.connected, "member": device.member, "blocked": device.blocked,
             "link_status": device.linkStatus, "input_reason": device.inputReason,
             "input_restart_required": device.inputRestartRequired, "remote_app_rdp": device.remoteAppRdp,
             "can_trigger_update": device.canTriggerUpdate, "input_path": name(device.inputPath),
             "input_paths_seen": device.inputPathsSeen.map(name), "input_path_paused": device.inputPathPaused,
             "routes": device.routes.map { route -> [String: Any] in
                 ["id": route.id, "endpoint": route.endpoint, "source": route.source, "status": route.status,
                  "active": route.active, "preferred": route.preferred, "failures": route.failures, "path": route.path,
                  "rtt_ms": nullable(route.rttMs), "last_probe_unix_ms": nullable(route.lastProbeUnixMs), "probe_error": nullable(route.probeError)]
             }]
        }
        out["pairing"] = s.pairing.map { ["id": $0.id, "peer_id": $0.peerId, "name": $0.name,
            "incoming": $0.incoming, "expires_at_ms": $0.expiresAtMs] as [String: Any] }
        out["pairing_message"] = nullable(s.pairingMessage)
        var features: [String: Any] = [:]
        for (key, page) in [("clipboard", Page.clipboard), ("files", .files), ("input", .input), ("mouse", .mouse), ("microphone", .microphone), ("remote_applications", .remoteApplications)] {
            features[key] = nullable(store.featureEnabled(for: page))
        }
        out["features"] = features
        out["transfers"] = s.transfers.map { ["id": $0.id, "name": $0.name, "detail": $0.detail,
            "phase": String(describing: $0.phase), "transferred": $0.transferred, "total": $0.total, "outgoing": $0.outgoing,
            "can_retry": $0.canRetry, "saved": $0.saved, "save_total": $0.saveTotal] as [String: Any] }
        out["files"] = ["enabled": s.files.enabled, "prefetch": s.files.prefetch, "threshold": s.files.threshold,
                        "maximum": s.files.maximum, "download_directory": s.files.downloadDirectory]
        out["cache"] = ["used": s.cache.used, "reserved": s.cache.reserved, "maximum": s.cache.maximum,
                        "active": s.cache.active, "completed": s.cache.completed, "message": s.cache.message]
        func permissionID(_ p: AppPermission) -> String {
            switch p { case .accessibility: "accessibility"; case .screenRecording: "screen_recording";
            case .inputMonitoring: "input_monitoring"; case .wifi: "wifi"; case .fullDiskAccess: "full_disk_access" }
        }
        out["permissions"] = ["items": AppPermission.allCases.map { p -> [String: Any] in
            ["id": permissionID(p), "title": p.title, "purpose": p.purpose,
             "granted": store.permissions.isGranted(p), "requires_relaunch": p.requiresRelaunch]
        }, "pending_relaunch": store.permissions.pendingRelaunch.map(permissionID)]
        let settings = s.input.settings
        let switchMode: String = switch settings.switchMode { case .instant: "instant"; case .dwell: "dwell"; case .doubleTap: "double_tap" }
        out["input"] = ["summary": s.input.summary, "error": nullable(s.input.error), "managed": s.input.managed,
            "managed_paused": s.input.managedPaused, "can_configure": s.input.canConfigure,
            "can_operate_remote_applications": s.input.canOperateRemoteApplications, "locked": s.input.locked,
            "recording": s.input.recording, "ssid": nullable(s.input.ssid), "observed_ssid": nullable(s.input.observedSsid),
            "environment_message": s.input.environmentMessage, "environment_conflict": s.input.environmentConflict,
            "candidates": s.input.candidates,
            "settings": ["feature_enabled": settings.featureEnabled, "enabled": settings.enabled, "controller": settings.controller,
                "participants": settings.participants, "switch_mode": switchMode, "disabled_corners": settings.disabledCorners,
                "corner_size": settings.cornerSize,
                "placements": settings.placements.map { ["device_id": $0.deviceId, "display_id": $0.displayId, "x": $0.x, "y": $0.y] as [String: Any] },
                "shortcuts": settings.shortcuts.map { ["key_code": $0.keyCode, "modifiers": $0.modifiers] as [String: Any] }],
            "displays": s.input.displays.map { ["id": $0.id, "device_id": $0.deviceId, "name": $0.name, "x": $0.x,
                "y": $0.y, "width": $0.width, "height": $0.height, "scale": $0.scale, "rotation": $0.rotation,
                "local": $0.local, "ready": $0.ready, "has_wallpaper": $0.wallpaperJpeg != nil,
                // 远端屏幕的预览由对端发布；本机屏幕走 input.wallpaper 按需截图。
                "wallpaper_jpeg": $0.wallpaperJpeg.map { $0.base64EncodedString() } ?? NSNull()] as [String: Any] },
            "quality": ["grade": s.input.quality.grade, "rtt_p50_ms": nullable(s.input.quality.rttP50Ms),
                "rtt_p95_ms": nullable(s.input.quality.rttP95Ms), "rtt_max_ms": nullable(s.input.quality.rttMaxMs),
                "hitches_per_minute": s.input.quality.hitchesPerMinute, "slow_injections": Int(s.input.quality.slowInjections),
                "samples": s.input.quality.samples.map { sample in
                    ["second": Int(sample.second), "rtt_ms": nullable(sample.rttMs), "hitches": Int(sample.hitches),
                     "hitch_network": Int(sample.hitchNetwork), "hitch_capture": Int(sample.hitchCapture)] as [String: Any]
                },
                "suggestions": s.input.quality.suggestions.map { suggestion in
                    ["id": suggestion.id, "priority": suggestion.priority, "trigger": suggestion.trigger,
                     "action": suggestion.action, "expected": suggestion.expected, "retest": suggestion.retest] as [String: Any]
                },
                "datagram_ready": nullable(s.input.quality.datagramReady)] as [String: Any],
            // Wi-Fi 场景规则与对端上报：原生「首选主控 / 对端上报」的数据来源。
            "rules": s.input.rules.map { ["ssid": $0.ssid, "controller": $0.controller] as [String: Any] },
            "peer_ssids": s.input.peerSsids.map { ["device_id": $0.deviceId, "ssid": $0.ssid] as [String: Any] },
            "remote_application_restriction": s.input.canOperateRemoteApplications ? "" : Self.remoteApplicationRestriction]
        // 远程应用管理器只在 macOS 14.2 起提供；低版本给出明确的关闭态。
        if #available(macOS 14.2, *) {
            // 预览与面板底部的运行状态：只保留最近 3 条，与原生 compact/常显一致。
        out["workflow_runs"] = WorkflowStore.shared.runs.prefix(3).map { run -> [String: Any] in
            ["id": run.id, "workflow_id": run.workflowId, "step_index": Int(run.stepIndex), "phase": run.phase, "message": run.message,
             "output_kind": String(describing: run.outputKind), "output_mime": run.outputMime,
             "output_paths": run.outputPaths, "output_bytes": String(run.outputBytes),
             "input_bytes": String(run.inputBytes), "width": Int(run.width), "height": Int(run.height)]
        }
        out["workflow_error"] = nullable(WorkflowStore.shared.error)
        out["remote_applications"] = ["windows_enabled": RemoteApplicationManager.shared.windowsEnabled,
                "error": nullable(RemoteApplicationManager.shared.error)]
        } else {
            out["remote_applications"] = ["windows_enabled": false, "error": NSNull()]
        }
        out["update"] = ["version": s.update.version, "build": String(s.update.build), "github": s.update.github,
            "release_url": nullable(s.update.releaseUrl), "checking": s.update.checking, "available": s.update.available,
            "manual_requested": s.update.manualRequested, "ready": s.update.ready, "downloading": s.update.downloading,
            "received_bytes": s.update.receivedBytes, "total_bytes": s.update.totalBytes, "target_version": nullable(s.update.targetVersion),
            "message": s.update.message, "error": nullable(s.update.error)]
        out["clipboard"] = ["enabled": s.clipboardEnabled, "error": nullable(s.clipboardError),
            "paste_target": nullable(ClipboardPanelController.shared.pasteTargetName),
            "automatic_ocr": UserDefaults.standard.object(forKey: ClipboardOCRIndexer.automaticIndexingKey) as? Bool
                ?? ClipboardOCRIndexer.defaultAutomaticIndexing,
            // 面板「查看全部」把查询条件交给主窗口页面；WebView 页面按 token 只应用一次。
            "navigation": store.clipboardNavigation.map { request in
                ["token": request.token.uuidString, "query": request.query, "source": nullable(request.source),
                 "list_id": nullable(request.listID), "selected_id": request.selectedID.map { String($0) } ?? NSNull()] as [String: Any]
            } as Any? ?? NSNull(),
            // 快照里的近期条目不参与分页，只用于“新复制立即上屏”和“历史是否为空”。
            "history": s.history.map { item in
                ["id": String(item.id), "preview": item.preview, "source": item.source,
                 "source_app_id": nullable(item.sourceAppId), "source_device_id": nullable(item.sourceDeviceId),
                 "copied_at_ms": nullable(item.copiedAtMs), "mime": item.mime, "length": String(item.length),
                 "has_image": item.hasImage, "file_count": Int(item.fileCount)] as [String: Any]
            }]
        let mouse = MouseButtonController.shared
        let g502Locations = G502ButtonLocations(
            json: UserDefaults.standard.string(forKey: "mouse.g502.physicalLocations.v1") ?? "{}")
        out["mouse"] = ["loaded": mouse.hasLoadedSettings, "enabled": mouse.isFeatureEnabled,
            "error": nullable(mouse.error), "hid_status": mouse.hidStatus,
            "tap_ready": mouse.tapReady, "save_status": mouse.saveStatus, "last_execution": mouse.lastExecution,
            "available_buttons": mouse.availableNativeButtons.sorted(),
            "bindings": try JSONSerialization.jsonObject(with: JSONEncoder().encode(mouse.settings.bindings)),
            // 平台原语：设备、电池与 SmartShift 状态。模型分组与展示由 Go 中间层完成。
            "devices": mouse.detectedMice.map { ["id": $0.id, "name": $0.name, "vendor_id": $0.vendorID,
                "product_id": $0.productID, "transport": $0.transport, "connected": true,
                "model": nullable($0.model?.id), "supported": $0.isSupported] as [String: Any] },
            "battery": mouse.batteryInfo.map {
                ["name": $0.name, "model": nullable(LogitechHIDProtocol.model($0.name)?.id),
                 "percent": $0.percent.map { Int($0) } ?? NSNull(), "charging": $0.charging] as [String: Any]
            },
            "wheel": mouse.wheelInfo.map {
                ["name": $0.name, "model": nullable(LogitechHIDProtocol.model($0.name)?.id),
                 "freespin": nullable($0.freespin)] as [String: Any]
            },
            "detecting": mouse.detecting, "capture_released": mouse.captureReleased,
            "last_button": nullable(mouse.lastButton),
            "g502_locations": Dictionary(uniqueKeysWithValues: G502ButtonPosition.allCases.map {
                ($0.rawValue, g502Locations.button(at: $0))
            })]
        let microphone = MicrophoneSharing.shared
        out["microphone"] = ["loaded": microphone.snapshot != nil, "enabled": microphone.isFeatureEnabled,
            "phase": nullable(microphone.snapshot?.phase), "detail": nullable(microphone.snapshot?.detail),
            "role": nullable(microphone.snapshot?.role), "peer": nullable(microphone.snapshot?.peer),
            "error": nullable(microphone.error ?? microphone.connectionError), "driver_installed": microphone.driverInstalled,
            "legacy_driver_installed": microphone.legacyDriverInstalled, "driver_message": nullable(microphone.driverMessage),
            "preferred_role": UserDefaults.standard.string(forKey: "microphone.role") ?? "",
            "managing_driver": microphone.managingDriver, "saving": microphone.saving,
            "inputs": microphone.inputs.map { ["id": $0.id, "name": $0.name] },
            "reader_apps": microphone.readerApps.map { ["id": String($0.id), "name": $0.name] },
            "is_default_input": nullable(microphone.isDefaultInput)]
        if let state = microphone.snapshot, var projection = out["microphone"] as? [String: Any] {
            projection["settings"] = ["enabled": state.settings.enabled, "source": state.settings.source,
                "allowed": state.settings.allowed, "input_uid": state.settings.input_uid]
            projection["native"] = ["driver": state.native.driver, "readers": state.native.readers,
                "streaming": state.native.streaming, "permission": state.native.permission, "level": state.native.level,
                "error": state.native.error]
            out["microphone"] = projection
        }
        out["workflows"] = ["enabled": store.workflowsEnabled]
        if let shelf = store.shelf { out["shelf"] = ["preparing": shelf.preparing,
            "files": shelf.files.map { ["path": $0.path, "name": $0.lastPathComponent] }] }
        return try JSONSerialization.data(withJSONObject: out, options: [.sortedKeys])
    }
}

@MainActor enum MyGoInvocation {
    static func run(action: String, params data: Data, store: AppStore) async throws -> Data {
        let params = (try JSONSerialization.jsonObject(with: data)) as? [String: Any] ?? [:]
        func string(_ key: String) -> String? { params[key] as? String }
        func object(_ value: some Encodable) throws -> Any {
            try JSONSerialization.jsonObject(with: JSONEncoder().encode(value))
        }
        func result(_ value: Any) throws -> Data { try JSONSerialization.data(withJSONObject: ["outcome": "applied", "value": value]) }
        func requiredEngine() throws -> Engine {
            guard let engine = store.engine else { throw MyGoBridgeError(code: "engine_unavailable", message: "引擎尚未就绪", retryable: true) }
            return engine
        }
        let command: EngineCommand?
        switch action {
        case "clipboard.updateRichText":
            guard let id = string("id").flatMap(UInt64.init), let text = string("text"), let client = store.client else {
                throw MyGoBridgeError(code: "invalid_params", message: "需要内容 ID、文本和已连接的服务")
            }
            let runs = try params["runs"].map { try JSONDecoder().decode([MyGoRichText.Run].self, from: JSONSerialization.data(withJSONObject: $0)) }
            let request = try MyGoRichText.mutation(id: id, text: text, runs: runs)
            return Data(try await client.libraryMutate(request).utf8)
        case "native.openPage", "native.open_page":
            guard let page = string("page") else { throw MyGoBridgeError(code: "invalid_params", message: "需要 page") }
            if let match = Page.allCases.first(where: { $0.bridgeID == page }), match != .settings {
                store.selectedPage = match
            }
            if let category = string("settings_category") {
                store.settingsDestination = SettingsDestination(tab: category, previous: store.settingsDestination)
            }
            MyGoUIHost.shared.present(page: page, settingsCategory: string("settings_category"))
            return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "mouse.scan":
            guard let enabled = params["enabled"] as? Bool else { throw MyGoBridgeError(code: "invalid_params", message: "需要 enabled") }
            if enabled { MouseButtonController.shared.beginDeviceScan() } else { MouseButtonController.shared.endDeviceScan() }
            return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "mouse.setMapping":
            guard let button = params["button"] as? Int, (0...255).contains(button), let modifiers = params["modifiers"] as? UInt64 else {
                throw MyGoBridgeError(code: "invalid_params", message: "按键或修饰键无效")
            }
            // 兼容旧桥接；新 WebView 路径由 Go 中间层校验并改走 mouse.applyBinding。
            var settings = MouseButtonController.shared.settings
            let action = try mouseAction(from: params["action"])
            if (208...218).contains(button) {
                let locations = g502Locations()
                if let position = locations.position(for: button), let generic = position.genericButton {
                    settings.bindings.removeAll { $0.button == generic }
                }
            }
            settings.set(button: button, modifiers: modifiers, action: action)
            MouseButtonController.shared.update(settings)
            return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "mouse.applyBinding":
            // 平台原语：只做类型校验与持久化。动作策略、G502 位置与
            // 通用键清理由 Go 中间层计算后通过 remove_buttons 传入。
            guard let button = params["button"] as? Int, (0...255).contains(button),
                  let modifiers = params["modifiers"] as? UInt64 else {
                throw MyGoBridgeError(code: "invalid_params", message: "按键或修饰键无效")
            }
            let action = try mouseAction(from: params["action"])
            var settings = MouseButtonController.shared.settings
            for removed in params["remove_buttons"] as? [Int] ?? [] where (0...255).contains(removed) {
                settings.bindings.removeAll { $0.button == removed }
            }
            settings.set(button: button, modifiers: modifiers, action: action)
            MouseButtonController.shared.update(settings)
            return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "mouse.locations":
            return try result(["g502": g502LocationMap(g502Locations())])
        case "mouse.saveLocations":
            guard let raw = params["g502"] as? [String: Any] else {
                throw MyGoBridgeError(code: "invalid_params", message: "需要 G502 位置映射")
            }
            var accepted: [String: Int] = [:]
            var used: Set<Int> = []
            for position in G502ButtonPosition.allCases {
                guard let number = raw[position.rawValue] as? Int, (208...218).contains(number),
                      used.insert(number).inserted else { continue }
                accepted[position.rawValue] = number
            }
            // 先用现有解码器验证并规范化，再写回 UserDefaults。
            let data = try JSONSerialization.data(withJSONObject: accepted, options: [.sortedKeys])
            let json = String(decoding: data, as: UTF8.self)
            let canonical = G502ButtonLocations(json: json)
            UserDefaults.standard.set(canonical.json, forKey: "mouse.g502.physicalLocations.v1")
            return try result(["g502": g502LocationMap(canonical)])
        case "mouse.setWheelMode":
            guard let name = string("name"), let freespin = params["freespin"] as? Bool else {
                throw MyGoBridgeError(code: "invalid_params", message: "需要设备名称和滚轮模式")
            }
            let applied = await MouseButtonController.shared.setWheelMode(name: name, freespin: freespin)
            return try result(["applied": applied])
        case "mouse.beginDetection":
            MouseButtonController.shared.beginDetection(g502: params["g502"] as? Bool ?? false)
            return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "mouse.endDetection":
            MouseButtonController.shared.endDetection()
            return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "mouse.clearCapture":
            MouseButtonController.shared.clearCapture()
            return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "mouse.locateG502":
            guard let position = string("position").flatMap(G502ButtonPosition.init(rawValue:)),
                  let button = params["button"] as? Int, (208...218).contains(button) else {
                throw MyGoBridgeError(code: "invalid_params", message: "需要位置和实体键编号")
            }
            var locations = g502Locations()
            locations.locate(button, at: position)
            UserDefaults.standard.set(locations.json, forKey: "mouse.g502.physicalLocations.v1")
            return try result(["g502": g502LocationMap(locations)])
        case "mouse.photo":
            guard let name = string("name"), MousePhotoAsset.names.contains(name) else {
                throw MyGoBridgeError(code: "invalid_params", message: "未知的鼠标图片")
            }
            guard let url = Bundle.main.url(forResource: name, withExtension: "png", subdirectory: "Mice"),
                  let data = try? Data(contentsOf: url) else {
                throw MyGoBridgeError(code: "native_error", message: "鼠标图片不可用")
            }
            return try result(["name": name, "data_base64": data.base64EncodedString()])
        case "mouse.contribute.start":
            mouseContributionProbe.start()
            return try mouseContributionState()
        case "mouse.contribute.stop":
            mouseContributionProbe.stop()
            return try mouseContributionState()
        case "mouse.contribute.state":
            return try mouseContributionState()
        case "mouse.contribute.clear":
            mouseContributionProbe.clear()
            return try mouseContributionState()
        case "mouse.contribute.remove":
            guard let number = params["number"] as? Int else { throw MyGoBridgeError(code: "invalid_params", message: "需要按键编号") }
            mouseContributionProbe.remove(number)
            return try mouseContributionState()
        case "mouse.contribute.rename":
            guard let number = params["number"] as? Int, let name = string("name") else {
                throw MyGoBridgeError(code: "invalid_params", message: "需要按键编号和名称")
            }
            mouseContributionProbe.rename(number, to: name)
            return try mouseContributionState()
        case "mouse.contribute.copy":
            guard let markdown = string("markdown") else { throw MyGoBridgeError(code: "invalid_params", message: "需要 Markdown") }
            let pasteboard = NSPasteboard.general
            pasteboard.clearContents()
            pasteboard.setString(markdown, forType: .string)
            return try result(["copied": true])
        case "mouse.contribute.save":
            guard let markdown = string("markdown"), let filename = string("filename") else {
                throw MyGoBridgeError(code: "invalid_params", message: "需要 Markdown 和文件名")
            }
            return try result(try await ClipboardItemActions.saveTextAwaiting(markdown, name: filename))
        case "microphone.speech.state":
            return try speechState()
        case "microphone.speech.prepare":
            await speechTester.prepare()
            return try speechState()
        case "microphone.speech.toggle":
            // 页面上已有的文字作为识别结果的追加前缀，与原生输入框一致。
            if let text = string("text") { speechTester.text = text }
            speechTester.toggle()
            return try speechState()
        case "microphone.speech.stop":
            if speechTester.listening { speechTester.finish() } else { speechTester.teardown() }
            return try speechState()
        case "microphone.speech.clear":
            speechTester.clear()
            return try speechState()
        case "microphone.speech.dismiss":
            speechTester.problem = nil
            return try speechState()
        case "microphone.speech.settings":
            guard let raw = string("url"), let url = URL(string: raw) else {
                throw MyGoBridgeError(code: "invalid_params", message: "需要系统设置链接")
            }
            NSWorkspace.shared.open(url)
            return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "mouse.reset":
            guard params["confirm"] as? Bool == true else { throw MyGoBridgeError(code: "confirmation_required", message: "恢复默认需要确认") }
            var settings = MouseSettings(); settings.enabled = MouseButtonController.shared.isFeatureEnabled
            MouseButtonController.shared.update(settings)
            return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "remoteApps.openPicker":
            guard let peer = string("device_id"), let device = store.snapshot?.devices.first(where: { $0.id == peer }) else {
                throw MyGoBridgeError(code: "invalid_params", message: "设备不存在")
            }
            guard #available(macOS 14.2, *) else { throw MyGoBridgeError(code: "capability_missing", message: "需要 macOS 14.2 或更新版本") }
            RemoteApplicationManager.shared.openPicker(peerID: peer, peerName: device.name)
            return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "remoteApps.catalog":
            guard #available(macOS 14.2, *), let peer = string("device_id") else { throw MyGoBridgeError(code: "invalid_params", message: "需要设备 ID 和 macOS 14.2") }
            return try await MyGoRemoteCatalog.catalog(peerID: peer, refresh: params["refresh"] as? Bool ?? false)
        case "remoteApps.closeCatalog":
            guard #available(macOS 14.2, *), let peer = string("device_id") else { throw MyGoBridgeError(code: "invalid_params", message: "需要设备 ID") }
            await MyGoRemoteCatalog.close(peerID: peer); return try result(true)
        case "remoteApps.open":
            guard #available(macOS 14.2, *), let peer = string("device_id"), let app = string("application_id") else { throw MyGoBridgeError(code: "invalid_params", message: "需要设备和应用 ID") }
            return try await MyGoRemoteCatalog.open(
                peerID: peer, applicationID: app, accountID: string("account_id"),
                username: string("username"), password: string("password"),
                saveAccount: params["save_account"] as? Bool ?? false, accountName: string("account_name") ?? "")
        case "remoteApps.setVirtualDisplay":
            guard let enabled = params["enabled"] as? Bool else { throw MyGoBridgeError(code: "invalid_params", message: "需要 enabled") }
            guard #available(macOS 14.2, *) else { throw MyGoBridgeError(code: "capability_missing", message: "需要 macOS 14.2 或更新版本") }
            MyGoRemoteCatalog.setVirtualDisplay(enabled: enabled)
            return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "remoteApps.setFavorite":
            guard #available(macOS 14.2, *), let peer = string("device_id"), let app = string("application_id") else {
                throw MyGoBridgeError(code: "invalid_params", message: "需要设备和应用 ID")
            }
            return try MyGoRemoteCatalog.setFavorite(peerID: peer, applicationID: app, favorite: params["favorite"] as? Bool ?? false)
        case "remoteApps.saveAccount":
            guard #available(macOS 14.2, *), let username = string("username"), let password = string("password") else { throw MyGoBridgeError(code: "invalid_params", message: "需要用户名和密码") }
            return try MyGoRemoteCatalog.saveAccount(id: string("id").flatMap(UUID.init(uuidString:)), name: string("name") ?? "", username: username, password: password)
        case "remoteApps.removeAccount":
            guard #available(macOS 14.2, *), let id = string("id").flatMap(UUID.init(uuidString:)), params["confirm"] as? Bool == true else { throw MyGoBridgeError(code: "confirmation_required", message: "删除登录账号需要确认") }
            MyGoRemoteCatalog.removeAccount(id: id); return try result(true)
        case "lanWeb.state":
            guard let client = store.client else { throw MyGoBridgeError(code: "engine_unavailable", message: "引擎尚未就绪") }
            return Data(try await client.lanWebState().utf8)
        case "lanWeb.qr":
            guard let address = string("address"), !address.isEmpty else { throw MyGoBridgeError(code: "invalid_params", message: "需要 address") }
            guard let image = MyGoLanWeb.qrCodeDataURL(for: address) else { throw MyGoBridgeError(code: "native_error", message: "无法生成二维码") }
            return try result(["image": image])
        case "lanWeb.configure":
            guard let client = store.client, let enabled = params["enabled"] as? Bool else { throw MyGoBridgeError(code: "invalid_params", message: "需要 enabled") }
            return Data(try await client.configureLanWeb(enabled: enabled, interfaceName: nil, port: 0).utf8)
        case "clipboard.peers":
            return try result((store.snapshot?.devices ?? []).filter { $0.trusted && !$0.blocked }.map { ["id": $0.id, "name": $0.name, "connected": $0.connected] as [String: Any] })
        case "clipboard.reveal":
            guard let id = string("id").flatMap(UInt64.init) else { throw MyGoBridgeError(code: "invalid_params", message: "内容 ID 无效") }
            ClipboardItemActions.revealInFinder(id); return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "clipboard.openInPreview":
            guard let id = string("id").flatMap(UInt64.init) else { throw MyGoBridgeError(code: "invalid_params", message: "内容 ID 无效") }
            ClipboardItemActions.openInPreview(id); return Data(#"{"outcome":"dispatched"}"#.utf8)
        // 保存、编辑、分享与工作流：WebView 只提供条目 id 与展示字段，原生执行原动作。
        case "clipboard.save":
            guard let id = string("id").flatMap(UInt64.init) else { throw MyGoBridgeError(code: "invalid_params", message: "内容 ID 无效") }
            let hasImage = params["has_image"] as? Bool ?? false
            let fileCount = params["file_count"] as? Int ?? 0
            guard MyGoUIHost.shared.isActive else {
                ClipboardItemActions.save(id: id, hasImage: hasImage, fileCount: fileCount)
                return Data(#"{"outcome":"dispatched"}"#.utf8)
            }
            // WebView 面板需要保存面板的真实结果：取消返回 saved=false，失败抛出错误。
            do {
                let outcome = try await ClipboardItemActions.saveAwaiting(id: id, hasImage: hasImage, fileCount: fileCount)
                return try result(outcome)
            } catch {
                let message = error.userFacingMessage
                store.error = message
                throw MyGoBridgeError(code: "native_error", message: message)
            }
        case "clipboard.edit":
            guard let id = string("id").flatMap(UInt64.init) else { throw MyGoBridgeError(code: "invalid_params", message: "内容 ID 无效") }
            ClipboardItemActions.edit(id: id, title: string("title"), hasImage: params["has_image"] as? Bool ?? false, fileCount: params["file_count"] as? Int ?? 0)
            return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "clipboard.share":
            guard let id = string("id").flatMap(UInt64.init), let peer = string("peer_id") else { throw MyGoBridgeError(code: "invalid_params", message: "需要内容与设备") }
            store.shareWorkflow(string("workflow_id"), historyID: id, peerID: peer)
            return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "workflow.run":
            guard let workflow = string("workflow_id"), let id = string("history_id").flatMap(UInt64.init) else { throw MyGoBridgeError(code: "invalid_params", message: "需要工作流与内容") }
            store.runWorkflow(workflow, historyID: id)
            return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "clipboard.appInfo":
            guard let bundleID = string("bundle_id"), !bundleID.isEmpty else { throw MyGoBridgeError(code: "invalid_params", message: "需要 bundle_id") }
            let image = ClipboardAppInfoCache.shared.icon(for: bundleID)
            var icon: Any = NSNull()
            if let image, let tiff = image.tiffRepresentation, let bitmap = NSBitmapImageRep(data: tiff),
               let png = bitmap.representation(using: .png, properties: [:]) {
                icon = "data:image/png;base64," + png.base64EncodedString()
            }
            return try result(["icon": icon,
                               "name": ClipboardAppInfoCache.shared.name(for: bundleID).map { $0 as Any } ?? NSNull()])
        case "clipboard.sources":
            let choices = ClipboardLibraryStore.shared.sourceChoices(legacyNames: store.snapshot?.history.map(\.source) ?? [], snapshot: store.snapshot)
            return try result(choices.map { ["id": $0.id, "name": $0.name] as [String: Any] })
        case "clipboard.requestFiles", "clipboard.stage":
            guard let id = string("id").flatMap(UInt64.init), let client = store.client else { throw MyGoBridgeError(code: "invalid_params", message: "内容 ID 无效") }
            _ = try await client.prepareClipboardPaste(id, session: UUID().uuidString)
            if action == "clipboard.stage" {
                let content = try await client.content(id)
                guard let shelf = store.shelf, !content.filePaths.isEmpty else { throw MyGoBridgeError(code: "native_error", message: "没有可暂存的本地文件") }
                shelf.append(content.filePaths.map { URL(fileURLWithPath: $0) })
            }
            return try result(true)
        case "microphone.refreshInputs": MicrophoneSharing.shared.refreshInputs(); return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "microphone.retry": MicrophoneSharing.shared.retryConnection(engine: store.engine); return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "microphone.chooseRole":
            guard let role = string("role"), ["provider", "receiver"].contains(role) else { throw MyGoBridgeError(code: "invalid_params", message: "身份无效") }
            if role == "provider", var settings = MicrophoneSharing.shared.snapshot?.settings, !settings.source.isEmpty {
                settings.source = ""; MicrophoneSharing.shared.configure(settings)
            }
            UserDefaults.standard.set(role, forKey: "microphone.role")
            return try result(true)
        case "microphone.configure":
            guard let raw = params["settings"] as? [String: Any], var settings = MicrophoneSharing.shared.snapshot?.settings else { throw MyGoBridgeError(code: "engine_unavailable", message: "麦克风设置尚未就绪") }
            if let source = raw["source"] as? String { settings.source = source }
            if let uid = raw["input_uid"] as? String { settings.input_uid = uid }
            if !settings.source.isEmpty, store.snapshot?.devices.contains(where: { $0.id == settings.source && $0.trusted && !$0.blocked }) != true { throw MyGoBridgeError(code: "invalid_params", message: "来源设备未配对") }
            MicrophoneSharing.shared.configure(settings)
            return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "microphone.setDefaultInput": MicrophoneSharing.shared.setDefaultInput(); return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "microphone.manageDriver":
            guard params["confirm"] as? Bool == true else { throw MyGoBridgeError(code: "confirmation_required", message: "驱动维护需要确认") }
            guard let raw = string("action"), let action = MicrophoneDriverManager.Action(rawValue: raw) else { throw MyGoBridgeError(code: "invalid_params", message: "驱动操作无效") }
            MicrophoneSharing.shared.manageDriver(action, restartAudio: params["restart_audio"] as? Bool ?? false)
            return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "system.choose_directory", "transfers.chooseFiles":
            let panel = NSOpenPanel()
            panel.canChooseDirectories = true
            panel.canChooseFiles = action == "transfers.chooseFiles"
            panel.allowsMultipleSelection = action == "transfers.chooseFiles"
            panel.title = string("title") ?? "选择文件"
            // WebView 模式下宿主是 accessory：先激活自己，面板才会浮到前面。
            NSApp.activate(ignoringOtherApps: true)
            let response: NSApplication.ModalResponse = await withCheckedContinuation { continuation in
                panel.begin { response in continuation.resume(returning: response) }
            }
            if action == "system.choose_directory" { return try result(response == .OK ? panel.urls.first?.path as Any? ?? NSNull() : NSNull()) }
            let files: [[String: Any]] = response == .OK ? panel.urls.map { url in
                let values = try? url.resourceValues(forKeys: [.fileSizeKey, .isDirectoryKey])
                return ["path": url.path, "name": url.lastPathComponent, "size": values?.fileSize ?? 0, "is_directory": values?.isDirectory ?? false]
            } : []
            return try result(files)
        case "login_item.set":
            guard let enabled = params["enabled"] as? Bool else { throw MyGoBridgeError(code: "invalid_params", message: "需要 enabled") }
            if enabled { try SMAppService.mainApp.register() } else { try await SMAppService.mainApp.unregister() }
            return try result(true)
        case "workflow.list":
            await WorkflowStore.shared.refresh()
            let definitions = try object(WorkflowStore.shared.definitions)
            let engine = try requiredEngine()
            let catalogJSON = try await Task.detached { try engine.workflowCatalogJson() }.value
            let catalog = try JSONSerialization.jsonObject(with: Data(catalogJSON.utf8))
            return try result(["definitions": definitions, "catalog": catalog])
        case "workflow.save":
            guard let draft = params["workflow"] as? [String: Any] else { throw MyGoBridgeError(code: "invalid_params", message: "缺少工作流") }
            let flow = try JSONDecoder().decode(WorkflowDefinition.self, from: JSONSerialization.data(withJSONObject: draft))
            try await WorkflowStore.shared.save(flow)
            return try result(true)
        case "workflow.delete":
            guard let id = string("id"), params["confirm"] as? Bool == true else { throw MyGoBridgeError(code: "confirmation_required", message: "删除工作流需要确认") }
            try await WorkflowStore.shared.delete(id)
            return try result(true)
        case "workflow.restore_conflict":
            guard let id = string("id") else { throw MyGoBridgeError(code: "invalid_params", message: "缺少工作流 ID") }
            try await WorkflowStore.shared.restoreConflict(id)
            return try result(true)
        case "workflow.cancel":
            guard let id = string("run_id") else { throw MyGoBridgeError(code: "invalid_params", message: "缺少任务 ID") }
            let engine = try requiredEngine()
            try await Task.detached { try engine.cancelWorkflow(runId: id) }.value
            return try result(true)
        case "workflow.system_shortcuts":
            let shortcuts = try await WorkflowStore.shared.systemShortcuts()
            return try result(shortcuts.map { ["id": $0.id, "name": $0.name] })
        case "clipboard.configure_shortcut":
            let value = params["shortcut"]
            let shortcut = try value.flatMap { $0 is NSNull ? nil : try JSONDecoder().decode(WorkflowKey.self, from: JSONSerialization.data(withJSONObject: $0)) }
            ClipboardPanelController.shared.configure(shortcut)
            return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "input.set_recording":
            guard let enabled = params["enabled"] as? Bool else { throw MyGoBridgeError(code: "invalid_params", message: "需要 enabled") }
            // 录制键盘组合时同时暂停剪贴板全局快捷键：它可能正是用户要录的组合。
            if params["suspend_shortcut"] as? Bool == true {
                ClipboardPanelController.shared.suspendShortcut(enabled)
            }
            try requiredEngine().setRecording(enabled: enabled)
            return try result(true)
        case "set_files":
            guard let raw = params["settings"] as? [String: Any], let enabled = raw["enabled"] as? Bool,
                let prefetch = raw["prefetch"] as? Bool, let threshold = raw["threshold"] as? NSNumber,
                let maximum = raw["maximum"] as? NSNumber, threshold.doubleValue >= 0, maximum.doubleValue >= 0,
                let directory = raw["download_directory"] as? String else { throw MyGoBridgeError(code: "invalid_params", message: "文件设置无效") }
            command = .setFiles(settings: FileSettings(enabled: enabled, prefetch: prefetch, threshold: threshold.uint64Value, maximum: maximum.uint64Value, downloadDirectory: directory))
        case "set_input", "input.setSettings":
            guard var settings = store.snapshot?.input.settings, let raw = params["settings"] as? [String: Any] else { throw MyGoBridgeError(code: "invalid_params", message: "键鼠设置无效") }
            if let value = raw["enabled"] as? Bool { settings.enabled = value }
            if let value = raw["controller"] as? Bool { settings.controller = value }
            if let value = raw["feature_enabled"] as? Bool { settings.featureEnabled = value }
            if let value = raw["participants"] as? [String] { settings.participants = value }
            if let value = raw["corner_size"] as? UInt16 { settings.cornerSize = value }
            if let value = raw["disabled_corners"] as? UInt8 { settings.disabledCorners = value }
            if let value = raw["switch_mode"] as? String {
                switch value { case "instant": settings.switchMode = .instant; case "dwell": settings.switchMode = .dwell; case "double_tap": settings.switchMode = .doubleTap; default: throw MyGoBridgeError(code: "invalid_params", message: "切换模式无效") }
            }
            if let values = raw["shortcuts"] as? [[String: Any]] {
                settings.shortcuts = try values.map { value in
                    guard let code = value["key_code"] as? UInt16, let mods = value["modifiers"] as? UInt64 else { throw MyGoBridgeError(code: "invalid_params", message: "快捷键无效") }
                    return Shortcut(keyCode: code, modifiers: mods)
                }
            }
            command = .setInput(settings: settings)
        case "transfers.inspectPaths":
            // Finder 拖入只给路径；补上名称、大小与目录标记，供发送区展示。
            guard let paths = params["paths"] as? [String], !paths.isEmpty else {
                throw MyGoBridgeError(code: "invalid_params", message: "没有拖入的文件")
            }
            let files: [[String: Any]] = paths.map { path in
                let url = URL(fileURLWithPath: path)
                let values = try? url.resourceValues(forKeys: [.fileSizeKey, .isDirectoryKey])
                return ["path": url.path, "name": url.lastPathComponent,
                        "size": values?.fileSize ?? 0, "is_directory": values?.isDirectory ?? false]
            }
            return try result(files)
        case "transfers.send":
            guard let peer = string("peer_id"), let paths = params["paths"] as? [String], !paths.isEmpty else { throw MyGoBridgeError(code: "invalid_params", message: "请选择文件和接收设备") }
            command = .sendFiles(peerId: peer, paths: paths)
        case "shelf.add":
            guard let paths = params["paths"] as? [String], !paths.isEmpty, let shelf = store.shelf else { throw MyGoBridgeError(code: "invalid_params", message: "请选择本地文件") }
            shelf.append(paths.map { URL(fileURLWithPath: $0) })
            if let error = store.error { throw MyGoBridgeError(code: "native_error", message: error) }
            return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "shelf.send":
            guard let peer = string("peer_id"), let shelf = store.shelf, !shelf.files.isEmpty else {
                throw MyGoBridgeError(code: "invalid_params", message: "暂存区没有可发送的文件")
            }
            command = .sendFiles(peerId: peer, paths: shelf.files.map(\.path))
        case "shelf.clear": store.shelf?.close(); return try result(true)
        case "shelf.remove":
            guard let path = string("path") else { throw MyGoBridgeError(code: "invalid_params", message: "缺少文件") }
            store.shelf?.remove([URL(fileURLWithPath: path)]); return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "devices.icon":
            // WebView 里没有 CoreTypes 资源与 SF Symbols：把原生同一个解析结果
            // 栅格化交给页面，图标与原生设备列表一致。
            guard let name = string("name"), !name.isEmpty else { throw MyGoBridgeError(code: "invalid_params", message: "缺少设备名称") }
            let pixels = min(256, max(32, (params["size"] as? Int) ?? 116))
            guard let png = DeviceProductIconResolver.png(
                for: name, modelIdentifier: string("model_identifier"), pixels: pixels
            ) else { return try result(["available": false]) }
            return try result(["available": true, "png_base64": png.base64EncodedString()])
        case "devices.connect": command = string("peer_id").map { .connect(peerId: $0) }
        case "devices.disconnect": command = string("peer_id").map { .disconnect(peerId: $0) }
        case "devices.pair": command = string("peer_id").map { .pair(peerId: $0) } ?? string("ip").map { .pairAt(ip: $0) }
        case "devices.accept": command = string("request_id").map { .approve(requestId: $0) }
        case "devices.reject": command = string("request_id").map { .reject(requestId: $0) }
        case "devices.forget":
            guard params["confirm"] as? Bool == true else { throw MyGoBridgeError(code: "confirmation_required", message: "忘记设备需要确认") }
            command = string("peer_id").map { .forget(peerId: $0) }
        case "devices.block": guard params["confirm"] as? Bool == true, let p = string("peer_id") else { throw MyGoBridgeError(code: "confirmation_required", message: "需要 confirm:true") }; command = .block(peerId: p)
        case "devices.unblock": command = string("peer_id").map { .allowPairing(peerId: $0) }
        case "devices.leaveNetwork":
            guard params["confirm"] as? Bool == true else { throw MyGoBridgeError(code: "confirmation_required", message: "退出设备网络需要确认") }
            command = .leaveNetwork
        case "devices.refresh", "refresh": command = .refreshDevices
        case "input.reclaim": store.reclaim(); return Data(#"{"status":"dispatched"}"#.utf8)
        case "updates.check": command = .checkUpdates
        case "download_and_install_update": command = .downloadAndInstallUpdate
        case "uninstall.plan":
            let plan = DeskFuseUninstaller.Plan.current()
            let ids = plan.availableComponents.map { uninstallID($0) }
            let defaults = plan.defaultSelection.map { uninstallID($0) }
            return try result(["components": ids, "default_selection": ids.filter { defaults.contains($0) },
                               "installed_by_homebrew": plan.installedByHomebrew])
        case "uninstall.perform":
            guard let raw = params["components"] as? [String] else {
                throw MyGoBridgeError(code: "invalid_params", message: "需要 components")
            }
            let plan = DeskFuseUninstaller.Plan.current()
            var selection: Set<DeskFuseUninstaller.Component> = []
            for id in raw {
                if let component = DeskFuseUninstaller.Component.allCases.first(where: { uninstallID($0) == id }) {
                    selection.insert(component)
                }
            }
            guard !selection.isEmpty else { throw MyGoBridgeError(code: "invalid_params", message: "没有可卸载的项") }
            let completed = try await DeskFuseUninstaller.perform(store: store, plan: plan, selection: selection)
            return try result(["completed": completed])
        case "permissions.refresh", "refresh_permissions": store.permissions.refresh(); command = .refreshPermissions
        case "permissions.open":
            if string("permission") == "microphone" {
                if string("mode") == "settings", let url = URL(string: "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone") { NSWorkspace.shared.open(url) }
                else { MicrophoneSharing.shared.requestPermission() }
                return Data(#"{"outcome":"dispatched"}"#.utf8)
            }
            let permission: AppPermission? = switch string("permission") {
            case "accessibility": .accessibility; case "screen_recording": .screenRecording
            case "input_monitoring": .inputMonitoring; case "wifi": .wifi; case "full_disk_access": .fullDiskAccess
            default: nil
            }
            guard let permission else { throw MyGoBridgeError(code: "invalid_params", message: "未知权限") }
            if string("mode") == "settings" { store.permissions.openSettings(permission) }
            else { store.permissions.authorize(permission) }
            return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "settings.set":
            guard let key = string("key"), let value = params["value"] as? Bool else { throw MyGoBridgeError(code: "invalid_params", message: "需要 key 与布尔 value") }
            switch key {
            case "keep_awake": store.keepAwake = value
            case "hide_dock_icon": store.hideDockIcon = value; UserDefaults.standard.set(value, forKey: AppStore.hideDockIconDefaultsKey); store.applyDockIconPolicy()
            case "workflows_enabled": store.workflowsEnabled = value
            case "automatic_ocr": UserDefaults.standard.set(value, forKey: ClipboardOCRIndexer.automaticIndexingKey)
            default: throw MyGoBridgeError(code: "unsupported_action", message: "不支持此设置")
            }
            return Data(#"{"outcome":"applied"}"#.utf8)
        case "support.copy_environment": SupportCenter.copyEnvironment(store); return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "support.export_diagnostics": SupportCenter.exportDiagnosticsInteractively(store); return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "support.reveal_logs": NSWorkspace.shared.open(SupportCenter.logsDirectory); return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "system.open_url", "system.openUrl":
            // http 仅用于本机引擎生成的局域网共享地址；其余 scheme 仍只允许 https/shortcuts。
            guard let raw = string("url"), let url = URL(string: raw), ["http", "https", "shortcuts"].contains(url.scheme?.lowercased() ?? "") else { throw MyGoBridgeError(code: "invalid_params", message: "不支持此链接") }
            guard NSWorkspace.shared.open(url) else { throw MyGoBridgeError(code: "native_error", message: "无法打开链接") }
            return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "login_item.status":
            let value: String = switch SMAppService.mainApp.status { case .enabled: "enabled"; case .requiresApproval: "requires_approval"; case .notRegistered: "not_registered"; case .notFound: "not_found"; @unknown default: "unknown" }
            return try JSONSerialization.data(withJSONObject: ["outcome": "applied", "value": value])
        case "login_item.open_settings": SMAppService.openSystemSettingsLoginItems(); return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "onboarding.restart":
            store.onboarding.restart()
            MyGoUIHost.shared.present(page: "onboarding")
            return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "onboarding.complete":
            if let features = params["features"] as? [String: Bool] {
                for (key, enabled) in features {
                    let page: Page? = switch key {
                    case "clipboard": .clipboard; case "files": .files; case "input": .input
                    case "mouse": .mouse; case "microphone": .microphone
                    case "remote_applications": .remoteApplications
                    default: nil
                    }
                    if let page, store.featureEnabled(for: page) != enabled {
                        store.setFeatureEnabled(enabled, for: page) {}
                    }
                }
            }
            store.onboarding.complete()
            MyGoUIHost.shared.present(page: "devices")
            return Data(#"{"outcome":"applied"}"#.utf8)
        case "transfers.cancel": command = string("id").map { .cancelTransfer(id: $0) }
        case "transfers.retry": command = string("id").map { .retryTransfer(id: $0) }
        // 配对确认由 WebView 配对窗口发起，命令仍由原生引擎执行。
        case "pairing.approve": command = string("id").map { .approve(requestId: $0) }
        case "pairing.reject": command = string("id").map { .reject(requestId: $0) }
        case "clear_cache": command = .clearCache
        case "devices.triggerUpdate": command = string("peer_id").map { .triggerPeerUpdate(peerId: $0) }
        case "devices.triggerUpdates": command = (params["peer_ids"] as? [String]).map { .triggerPeerUpdates(peerIds: $0) }
        case "devices.setInputPath":
            let path: InputPath? = switch string("path") { case "auto": .auto; case "thunderbolt": .thunderbolt; case "ethernet": .ethernet; case "wifi": .wiFi; default: nil }
            guard let peer = string("device_id"), let path else { throw MyGoBridgeError(code: "invalid_params", message: "设备或线路无效") }
            command = .setInputPath(deviceId: peer, path: path)
        case "input.wallpaper":
            guard let id = string("display_id") else { throw MyGoBridgeError(code: "invalid_params", message: "需要 display_id") }
            // 原生页面在出现时截取本机屏幕预览；WebView 版按需触发同一次截取。
            // 截图要经过 ScreenCaptureKit，不能在一次请求里等它完成：先把缓存返回，
            // 未命中时在后台触发，页面稍后重试。
            if store.localDisplayWallpapers[id] == nil {
                Task { @MainActor in await store.refreshDisplayWallpapers() }
                return try result(["image": NSNull()])
            }
            guard let image = store.localDisplayWallpapers[id] else { return try result(["image": NSNull()]) }
            var rect = CGRect(origin: .zero, size: image.size)
            guard let cgImage = image.cgImage(forProposedRect: &rect, context: nil, hints: nil),
                  let jpeg = NSBitmapImageRep(cgImage: cgImage)
                    .representation(using: .jpeg, properties: [.compressionFactor: 0.7]) else {
                return try result(["image": NSNull()])
            }
            return try result(["image": "data:image/jpeg;base64," + jpeg.base64EncodedString()])
        case "input.previewDisplayMove":
            guard let device = string("device_id"), let rows = params["displays"] as? [[String: Any]], !rows.isEmpty else {
                throw MyGoBridgeError(code: "invalid_params", message: "需要设备与屏幕")
            }
            func number(_ row: [String: Any], _ key: String, _ fallback: Double = 0) -> Double {
                (row[key] as? NSNumber)?.doubleValue ?? fallback
            }
            let displays = try rows.map { row -> Display in
                guard let id = row["id"] as? String, let deviceID = row["device_id"] as? String else {
                    throw MyGoBridgeError(code: "invalid_params", message: "屏幕缺少 id/device_id")
                }
                return Display(id: id, deviceId: deviceID, name: row["name"] as? String ?? "",
                               x: number(row, "x"), y: number(row, "y"), width: number(row, "width"), height: number(row, "height"),
                               scale: number(row, "scale", 1),
                               rotation: UInt16(max(0, min(65535, Int(number(row, "rotation"))))),
                               local: row["local"] as? Bool ?? false, ready: row["ready"] as? Bool ?? true, wallpaperJpeg: nil)
            }
            let offset = try previewDisplayMove(displays: displays, deviceId: device,
                deltaX: number(params, "delta_x"), deltaY: number(params, "delta_y"),
                snapDistance: number(params, "snap_distance", 14), attach: params["attach"] as? Bool ?? true)
            return try result(["x": offset.x, "y": offset.y])
        case "input.setParticipant":
            guard let id = string("device_id"), let included = params["included"] as? Bool,
                  var settings = store.snapshot?.input.settings else { throw MyGoBridgeError(code: "invalid_params", message: "需要 device_id 与 included") }
            if included {
                guard store.snapshot?.devices.contains(where: { $0.id == id && $0.member && !$0.blocked }) == true else {
                    throw MyGoBridgeError(code: "invalid_params", message: "设备尚未配对")
                }
                if !settings.participants.contains(id) { settings.participants.append(id) }
                store.allowAutoJoin(id)
            } else {
                settings.participants.removeAll { $0 == id }
                store.suppressAutoJoin(id)
            }
            // 保存的屏幕位置随设备保留，重新添加时恢复原位。
            command = .setInput(settings: settings)
        case "input.chooseController":
            guard let id = string("id") else { throw MyGoBridgeError(code: "invalid_params", message: "需要 id") }
            command = .chooseController(id: id, remember: params["remember"] as? Bool ?? false)
        case "input.reconnect": command = string("device_id").map { .restartInputActivation(peerId: $0) }
        case "input.moveDevice":
            guard let peer = string("deviceId"), let display = string("displayId"), let x = params["x"] as? Double, let y = params["y"] as? Double, x.isFinite, y.isFinite else { throw MyGoBridgeError(code: "invalid_params", message: "屏幕位置无效") }
            command = .moveInputDevice(deviceId: peer, displayId: display, x: x, y: y)
        case "clipboard.copy", "clipboard.paste":
            guard let raw = string("id"), let id = UInt64(raw) else { throw MyGoBridgeError(code: "invalid_params", message: "需要 numeric id") }
            let plain = params["plain_text"] as? Bool ?? false
            guard MyGoUIHost.shared.isActive else {
                ClipboardItemActions.copy(id, plainText: plain, paste: action == "clipboard.paste")
                return Data(#"{"outcome":"dispatched"}"#.utf8)
            }
            // WebView 面板需要真实的完成结果：失败要能提示，复制后要能关闭面板。
            do {
                let changeCount = try await ClipboardItemActions.performAwaiting(id: id, plainText: plain, paste: action == "clipboard.paste")
                return try result(["change_count": changeCount])
            } catch {
                throw MyGoBridgeError(code: "native_error", message: error.localizedDescription)
            }
        case "panel.hide": MyGoUIHost.shared.hidePanel(); return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "app.showMain": store.openMain(); return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "app.quit": NSApp.terminate(nil); return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "clipboard.showInMain":
            let selected = string("selected_id").flatMap(UInt64.init)
            store.clipboardNavigation = ClipboardNavigationRequest(
                query: string("query") ?? "", source: string("source"), selectedID: selected, listID: string("list_id"))
            store.selectedPage = .clipboard
            store.openMain()
            return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "clipboard.chooseApp", "clipboard.choose_app":
            return try result(["bundle_id": ClipboardLibraryAppPicker.choose().map { $0 as Any } ?? NSNull()])
        case "clipboard.indexOCR":
            guard let ids = params["ids"] as? [String] else { throw MyGoBridgeError(code: "invalid_params", message: "需要 ids") }
            for raw in ids { if let id = UInt64(raw) { ClipboardOCRIndexer.shared.schedule(id, client: store.client) } }
            return Data(#"{"outcome":"applied"}"#.utf8)
        case "clipboard.cancelPreparation":
            guard let session = string("session"), let client = store.client else { throw MyGoBridgeError(code: "invalid_params", message: "需要 session") }
            try await client.cancelClipboardPreparation(session)
            return Data(#"{"outcome":"applied"}"#.utf8)
        case "clipboard.showPanel", "clipboard.togglePanel":
            ClipboardPanelController.shared.toggle()
            return Data(#"{"outcome":"dispatched"}"#.utf8)
        case "shelf.show": store.shelf?.show(); return Data(#"{"status":"dispatched"}"#.utf8)
        case "support.report", "support.reportProblem": SupportCenter.reportProblem(store, error: string("error")); return Data(#"{"status":"dispatched"}"#.utf8)
        case "feature.setEnabled":
            guard let feature = string("feature"), let enabled = params["enabled"] as? Bool else { throw MyGoBridgeError(code: "invalid_params", message: "需要 feature 和 enabled") }
            switch feature {
            case "clipboard": command = .setClipboard(enabled: enabled)
            case "files": guard var settings = store.snapshot?.files else { throw MyGoBridgeError(code: "engine_unavailable", message: "引擎尚未就绪") }; settings.enabled = enabled; command = .setFiles(settings: settings)
            case "input": guard var settings = store.snapshot?.input.settings else { throw MyGoBridgeError(code: "engine_unavailable", message: "引擎尚未就绪") }; settings.featureEnabled = enabled; command = .setInput(settings: settings)
            case "mouse": guard MouseButtonController.shared.hasLoadedSettings else { throw MyGoBridgeError(code: "engine_unavailable", message: "鼠标设置尚未就绪") }; MouseButtonController.shared.setFeatureEnabled(enabled); return Data(#"{"outcome":"dispatched"}"#.utf8)
            case "microphone": guard MicrophoneSharing.shared.snapshot != nil else { throw MyGoBridgeError(code: "engine_unavailable", message: "麦克风设置尚未就绪") }; MicrophoneSharing.shared.setFeatureEnabled(enabled); return Data(#"{"outcome":"dispatched"}"#.utf8)
            case "remote_applications": guard #available(macOS 14.2, *) else { throw MyGoBridgeError(code: "capability_missing", message: "需要 macOS 14.2 或更新版本") }; RemoteApplicationManager.shared.enabled = enabled; return Data(#"{"outcome":"applied"}"#.utf8)
            case "windows_remote_applications":
                guard #available(macOS 14.2, *) else { throw MyGoBridgeError(code: "capability_missing", message: "需要 macOS 14.2 或更新版本") }
                RemoteApplicationManager.shared.windowsEnabled = enabled
                return Data(#"{"outcome":"applied"}"#.utf8)
            default: throw MyGoBridgeError(code: "unsupported_action", message: action)
            }
        default: throw MyGoBridgeError(code: "unsupported_action", message: action)
        }
        guard let command else { throw MyGoBridgeError(code: "invalid_params", message: "缺少动作参数") }; guard store.client != nil else { throw MyGoBridgeError(code: "engine_unavailable", message: "引擎尚未就绪", retryable: true) }
        try await MyGoCommandWaiter.submit(command, store: store)
        return try JSONSerialization.data(withJSONObject: ["outcome": "applied"])
    }

    /// 平台原语只校验动作结构；具体策略在 Go 中间层。
    private static func mouseAction(from raw: Any?) throws -> MouseAction? {
        guard let raw, !(raw is NSNull) else { return nil }
        let action = try JSONDecoder().decode(MouseAction.self, from: JSONSerialization.data(withJSONObject: raw))
        guard ["disabled", "mouse", "shortcut"].contains(action.kind) else {
            throw MyGoBridgeError(code: "invalid_params", message: "按键动作无效")
        }
        if action.kind == "mouse", action.button == nil {
            throw MyGoBridgeError(code: "invalid_params", message: "缺少目标按键")
        }
        if action.kind == "shortcut", action.key_code == nil {
            throw MyGoBridgeError(code: "invalid_params", message: "缺少快捷键")
        }
        return action
    }

    /// WebView 使用 snake_case 组件 ID，与权限列表的命名一致。
    private static func uninstallID(_ component: DeskFuseUninstaller.Component) -> String {
        component.rawValue.reduce(into: "") { result, character in
            if character.isUppercase {
                result.append("_")
                result.append(character.lowercased())
            } else {
                result.append(character)
            }
        }
    }

    private static func g502Locations() -> G502ButtonLocations {
        G502ButtonLocations(json: UserDefaults.standard.string(forKey: "mouse.g502.physicalLocations.v1") ?? "{}")
    }

    private static func g502LocationMap(_ locations: G502ButtonLocations) -> [String: Int] {
        Dictionary(uniqueKeysWithValues: G502ButtonPosition.allCases.map { ($0.rawValue, locations.button(at: $0)) })
    }

    private static let mouseContributionProbe = MouseButtonProbe()

    /// 语音输入测试的状态机只有一份：原生页与 WebView 页切换时不会产生并发录音。
    private static let speechTester = MicrophoneSpeechTester()

    private static func speechState() throws -> Data {
        let phase: String = switch speechTester.phase {
        case .loading: "loading"; case .idle: "idle"; case .authorizing: "authorizing"; case .listening: "listening"
        }
        var value: [String: Any] = [
            "phase": phase,
            "listening": speechTester.listening,
            "text": speechTester.text,
            "level": Double(speechTester.level),
            "input_name": speechTester.inputName.map { $0 as Any } ?? NSNull(),
            "on_device": speechTester.onDevice,
        ]
        if let problem = speechTester.problem {
            value["problem"] = ["title": problem.title, "message": problem.message,
                                "settings_url": problem.settingsURL?.absoluteString ?? NSNull()]
        } else {
            value["problem"] = NSNull()
        }
        return try JSONSerialization.data(withJSONObject: ["outcome": "applied", "value": value])
    }

    /// 提供鼠标适配数据的探针状态与采集环境；Markdown 由 Go 中间层生成。
    private static func mouseContributionState() throws -> Data {
        let info = Bundle.main.infoDictionary ?? [:]
        let value: [String: Any] = [
            "capturing": mouseContributionProbe.capturing,
            "samples": mouseContributionProbe.samples.map {
                ["number": $0.number, "name": $0.name, "count": $0.count] as [String: Any]
            },
            "last_number": mouseContributionProbe.lastNumber.map { $0 } ?? NSNull(),
            "error": mouseContributionProbe.error.map { $0 } ?? NSNull(),
            "app_version": info["CFBundleShortVersionString"] as? String ?? "开发版",
            "app_build": info["CFBundleVersion"] as? String ?? "",
            "system_version": ProcessInfo.processInfo.operatingSystemVersionString,
        ]
        return try JSONSerialization.data(withJSONObject: ["outcome": "applied", "value": value])
    }
}

enum MousePhotoAsset {
    static let names: Set<String> = ["g502-lightspeed-top", "g502-lightspeed-side",
                                     "mx-master-3-top", "mx-master-3-side"]
}

@MainActor private final class MyGoCommandWaiter {
    private var continuation: CheckedContinuation<Void, Error>?
    private var timer: Task<Void, Never>?

    private func finish(_ result: Result<Void, Error>) {
        guard let continuation else { return }
        self.continuation = nil
        timer?.cancel()
        timer = nil
        continuation.resume(with: result)
    }

    static func submit(_ command: EngineCommand, store: AppStore) async throws {
        let waiter = MyGoCommandWaiter()
        try await withTaskCancellationHandler {
            try Task.checkCancellation()
            try await withCheckedThrowingContinuation { continuation in
                waiter.continuation = continuation
                waiter.timer = Task {
                    do { try await Task.sleep(for: .seconds(10)) }
                    catch { return }
                    waiter.finish(.failure(MyGoBridgeError(code: "timeout", message: "操作仍未确认，请检查最新状态后重试")))
                }
                store.send(command, applied: { waiter.finish(.success(())) }, failed: { message in
                    waiter.finish(.failure(MyGoBridgeError(code: "engine_error", message: message)))
                })
            }
        } onCancel: {
            Task { @MainActor in waiter.finish(.failure(CancellationError())) }
        }
    }
}

typealias MyGoEngineProvider = @Sendable () async -> (any MyGoLibraryQuerying)?
typealias MyGoSnapshotProvider = @Sendable () async throws -> Data
typealias MyGoInvokeHandler = @Sendable (String, Data) async throws -> Data

/// Decodes framed requests, runs them against the engine and writes framed
/// replies. Transport-agnostic: bytes in via `receive`, framed replies out via `send`.
actor MyGoBridgeServer {
    private let engine: MyGoEngineProvider
    private let send: @Sendable (Data) -> Void
    private let onClose: @Sendable () -> Void
    private let snapshot: MyGoSnapshotProvider?
    private let invoke: MyGoInvokeHandler?
    private var reader = MyGoFrameReader()
    private var inFlight: [String: Task<Void, Never>] = [:]
    private var closed = false

    /// `send` receives complete frames and must preserve call order.
    init(engine: @escaping MyGoEngineProvider, send: @escaping @Sendable (Data) -> Void, onClose: @escaping @Sendable () -> Void = {}, snapshot: MyGoSnapshotProvider? = nil, invoke: MyGoInvokeHandler? = nil) {
        self.engine = engine
        self.send = send
        self.onClose = onClose
        self.snapshot = snapshot
        self.invoke = invoke
    }

    func receive(_ data: Data) {
        guard !closed else { return }
        reader.append(data)
        do {
            while let payload = try reader.nextFrame() { handle(payload) }
        } catch let error as MyGoBridgeError {
            reply(MyGoBridgeReply.failure(error))
            close()
        } catch {
            close()
        }
    }

    /// The peer closed its end or the app is quitting. In-flight queries are cancelled
    /// (which interrupts the SQL) and produce no reply.
    func close() {
        guard !closed else { return }
        closed = true
        for task in inFlight.values { task.cancel() }
        inFlight.removeAll()
        onClose()
    }

    private func reply(_ payload: Data) { send(MyGoBridgeFrame.encode(payload)) }

    private func handle(_ payload: Data) {
        let request: MyGoBridgeRequest
        do { request = try MyGoBridgeRequestParser.parse(payload) } catch {
            reply(MyGoBridgeReply.failure(error as? MyGoBridgeError ?? MyGoBridgeError(code: "internal", message: error.localizedDescription)))
            return
        }
        switch request.command {
        case .hello:
            replySuccess(id: request.id, result: MyGoBridgeReply.helloResult)
        case .cancel(let target):
            let found = inFlight[target]
            found?.cancel()
            replySuccess(id: request.id, result: Data(#"{"cancelled":\#(found != nil)}"#.utf8))
        case .libraryQuery(let json):
            guard inFlight[request.id] == nil else {
                reply(MyGoBridgeReply.failure(MyGoBridgeError(code: "duplicate_id", message: "id 正在处理中", id: request.id)))
                return
            }
            guard inFlight.count < MyGoBridge.maxInFlight else {
                reply(MyGoBridgeReply.failure(MyGoBridgeError(code: "busy", message: "并发请求过多", retryable: true, id: request.id)))
                return
            }
            // handle() is synchronous on the actor, so the entry exists before the task can finish.
            inFlight[request.id] = Task { await self.runQuery(id: request.id, json: json) }
        case .libraryMutate(let json): enqueue(request.id) { await self.runMutation(id: request.id, json: json) }
        case .appSnapshot: enqueue(request.id, serial: false) { await self.runSnapshot(id: request.id) }
        case .clipboardContent(let itemID, let image, let rtf): enqueue(request.id, serial: false) { await self.runContent(id: request.id, itemID: itemID, image: image, rtf: rtf) }
        case .clipboardThumbnail(let itemID, let pixel): enqueue(request.id, serial: false) { await self.runThumbnail(id: request.id, itemID: itemID, pixel: pixel) }
        case .appInvoke(let action, let params): enqueue(request.id) { await self.runInvoke(id: request.id, action: action, params: params) }
        }
    }

    private var serialTail: Task<Void, Never>?
    private func enqueue(_ id: String, serial: Bool = true, _ operation: @escaping @Sendable () async -> Void) {
        guard inFlight[id] == nil else { reply(MyGoBridgeReply.failure(MyGoBridgeError(code: "duplicate_id", message: "id 正在处理中", id: id))); return }
        guard inFlight.count < MyGoBridge.maxInFlight else { reply(MyGoBridgeReply.failure(MyGoBridgeError(code: "busy", message: "并发请求过多", retryable: true, id: id))); return }
        let previous = serial ? serialTail : nil
        let task = Task { [weak self] in
            _ = await previous?.value
            guard let self else { return }
            if Task.isCancelled {
                await self.cancelQueued(id: id)
                return
            }
            await operation()
            await self.clearTail(id: id)
        }
        if serial { serialTail = task }
        inFlight[id] = task
    }
    private func cancelQueued(id: String) {
        inFlight[id] = nil
        if !closed { reply(MyGoBridgeReply.failure(MyGoBridgeError(code: "cancelled", message: "请求已取消", id: id))) }
    }
    private func clearTail(id: String) { inFlight[id] = nil }

    private func replySuccess(id: String, result: Data) {
        do { reply(try MyGoBridgeReply.success(id: id, result: result)) } catch {
            reply(MyGoBridgeReply.failure(error as? MyGoBridgeError ?? MyGoBridgeError(code: "internal", message: "回复编码失败", id: id)))
        }
    }

    private func runQuery(id: String, json: String) async {
        let outcome: Data
        do {
            guard let client = await engine() else {
                throw MyGoBridgeError(code: "engine_unavailable", message: "引擎尚未就绪", retryable: true, id: id)
            }
            let result = try await client.libraryQuery(json)
            outcome = try MyGoBridgeReply.success(id: id, result: Data(result.utf8))
        } catch let error as MyGoBridgeError {
            outcome = MyGoBridgeReply.failure(error)
        } catch {
            // A cancelled task can surface as an engine error when SQLite was interrupted.
            let cancelled = Task.isCancelled || error is CancellationError
            outcome = MyGoBridgeReply.failure(MyGoBridgeError(
                code: cancelled ? "cancelled" : "engine_error",
                message: cancelled ? "请求已取消" : error.userFacingMessage, id: id))
        }
        inFlight[id] = nil
        if !closed { reply(outcome) }
    }

    private func finish(_ id: String, _ result: Result<Data, Error>) {
        guard !closed else { inFlight[id] = nil; return }
        switch result { case .success(let data): replySuccess(id: id, result: data); case .failure(let error as MyGoBridgeError): reply(MyGoBridgeReply.failure(MyGoBridgeError(code: error.code, message: error.message, retryable: error.retryable, id: id))); case .failure(let error): reply(MyGoBridgeReply.failure(MyGoBridgeError(code: "engine_error", message: error.userFacingMessage, id: id))) }
    }
    private func runMutation(id: String, json: String) async { do { guard let c = await engine() else { throw MyGoBridgeError(code: "engine_unavailable", message: "引擎尚未就绪", retryable: true, id: id) }; finish(id, .success(Data(try await c.libraryMutate(json).utf8))) } catch { finish(id, .failure(error)) } }
    private func runSnapshot(id: String) async { do { guard let snapshot else { throw MyGoBridgeError(code: "unsupported_action", message: "snapshot unavailable", id: id) }; finish(id, .success(try await snapshot())) } catch { finish(id, .failure(error)) } }
    private func runContent(id: String, itemID: UInt64, image: Bool, rtf: Bool) async {
        do {
            guard let c = await engine() else { throw MyGoBridgeError(code: "engine_unavailable", message: "引擎尚未就绪", retryable: true, id: id) }
            let value = try await c.content(itemID)
            var object: [String: Any] = ["id": String(value.id), "text": value.text,
                "truncated": value.truncated, "file_paths": value.filePaths, "rtf_available": !value.rtf.isEmpty,
                "has_image": !value.imagePng.isEmpty, "can_request_files": value.canRequestFiles]
            if let runs = await MyGoRichText.runs(from: value.rtf) {
                object["rich_runs"] = try JSONSerialization.jsonObject(with: JSONEncoder().encode(runs))
            }
            if image { object["image_png_base64"] = value.imagePng.base64EncodedString() }
            if rtf { object["rtf_base64"] = value.rtf.base64EncodedString() }
            finish(id, .success(try JSONSerialization.data(withJSONObject: object)))
        } catch { finish(id, .failure(error)) }
    }
    private func runThumbnail(id: String, itemID: UInt64, pixel: Int) async { do { guard let c = await engine() else { throw MyGoBridgeError(code: "engine_unavailable", message: "引擎尚未就绪", retryable: true, id: id) }; let d = try await c.thumbnail(itemID, maxPixelSize: pixel); finish(id, .success(try JSONSerialization.data(withJSONObject: ["data_base64": d.base64EncodedString(), "mime": "image/png"]))) } catch { finish(id, .failure(error)) } }
    private func runInvoke(id: String, action: String, params: Data) async { do { guard let invoke else { throw MyGoBridgeError(code: "unsupported_action", message: action, id: id) }; finish(id, .success(try await invoke(action, params))) } catch { finish(id, .failure(error)) } }
}
