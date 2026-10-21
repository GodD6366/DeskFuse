import Testing
@testable import DeskFuse

struct InputWifiIdentityTests {
    private let names = ["mbp": "godd-MBP-108", "claw": "godClawMBP-2"]

    @Test func missingSsidDistinguishesPermissionFromWiredNetwork() {
        let denied = InputWifiIdentity(observed: nil, effective: nil, rules: [], peerSsids: [],
                                       names: names, localId: "local", wifiGranted: false)
        #expect(denied.local == .needsPermission)
        let wired = InputWifiIdentity(observed: nil, effective: nil, rules: [], peerSsids: [],
                                      names: names, localId: "local", wifiGranted: true)
        #expect(wired.local == .notOnWifi)
    }

    @Test func adoptedPeerSsidResolvesPreferredControllerByName() {
        let identity = InputWifiIdentity(
            observed: nil, effective: "Youzankeji-office",
            rules: [(ssid: "Youzankeji-office", controller: "mbp"), (ssid: "Home", controller: "local")],
            peerSsids: [(deviceId: "mbp", ssid: "Youzankeji-office")],
            names: names, localId: "local", wifiGranted: false)
        #expect(identity.adoptedFromPeer)
        #expect(identity.preferredController == "godd-MBP-108")
        #expect(identity.peers.map(\.matches) == [true])
    }

    @Test func peersAreComparedWithTheEffectiveSsid() {
        let identity = InputWifiIdentity(
            observed: "Home", effective: "Home",
            rules: [(ssid: "Home", controller: "local")],
            peerSsids: [(deviceId: "mbp", ssid: "Home"), (deviceId: "claw", ssid: "Office-5G"), (deviceId: "unknown-device", ssid: "Home")],
            names: names, localId: "local", wifiGranted: true)
        #expect(!identity.adoptedFromPeer)
        #expect(identity.preferredController == "这台 Mac")
        #expect(identity.peers.map(\.name) == ["godClawMBP-2", "godd-MBP-108", "unknown-"])
        #expect(identity.peers.map(\.matches) == [false, true, true])
    }

    @Test func peersHaveNoVerdictWithoutAnyLocalReference() {
        let identity = InputWifiIdentity(
            observed: nil, effective: nil, rules: [],
            peerSsids: [(deviceId: "mbp", ssid: "A"), (deviceId: "claw", ssid: "B")],
            names: names, localId: "local", wifiGranted: true)
        #expect(identity.preferredController == nil)
        #expect(identity.peers.allSatisfy { $0.matches == nil })
    }

    @Test func summaryPrefersEffectiveSsidAndFlagsAttention() {
        let denied = InputWifiIdentity(observed: nil, effective: nil, rules: [], peerSsids: [],
                                       names: names, localId: "local", wifiGranted: false)
        #expect(denied.summary == "未读取 Wi-Fi")
        #expect(denied.needsAttention)
        let adopted = InputWifiIdentity(observed: nil, effective: "Home", rules: [],
                                        peerSsids: [(deviceId: "mbp", ssid: "Home")],
                                        names: names, localId: "local", wifiGranted: true)
        #expect(adopted.summary == "Home")
        #expect(!adopted.needsAttention)
        let mismatch = InputWifiIdentity(observed: "Home", effective: "Home", rules: [],
                                         peerSsids: [(deviceId: "claw", ssid: "Office")],
                                         names: names, localId: "local", wifiGranted: true)
        #expect(mismatch.needsAttention)
    }
}
