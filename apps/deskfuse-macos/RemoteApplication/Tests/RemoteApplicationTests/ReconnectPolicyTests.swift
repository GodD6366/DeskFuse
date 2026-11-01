import Foundation
import Testing
@testable import DeskFuseRemoteApplication

@Suite("RDP reconnect policy")
struct ReconnectPolicyTests {
    @Test func backoffWalksTheScheduleAndRepeatsTheLastDelay() {
        let policy = RemoteRdpReconnectPolicy()
        #expect(policy.delay(afterAttempt: 1) == 1)
        #expect(policy.delay(afterAttempt: 2) == 2)
        #expect(policy.delay(afterAttempt: 3) == 4)
        #expect(policy.delay(afterAttempt: 4) == 8)
        #expect(policy.delay(afterAttempt: 5) == 8)
        #expect(policy.delay(afterAttempt: 100) == 8)
    }

    @Test func degenerateAttemptNumbersDoNotTrap() {
        let policy = RemoteRdpReconnectPolicy(retryDelays: [3, 6])
        #expect(policy.delay(afterAttempt: 0) == 3)
        #expect(policy.delay(afterAttempt: -5) == 3)
        #expect(policy.delay(afterAttempt: 3) == 6)
    }

    @Test func emptyScheduleStillHasADeadline() {
        let policy = RemoteRdpReconnectPolicy(deadline: 10, retryDelays: [])
        #expect(policy.delay(afterAttempt: 4) == 0)
        #expect(policy.mayRetry(elapsed: 9.9))
        #expect(!policy.mayRetry(elapsed: 10))
    }

    @Test func defaultBudgetMatchesDocumentedMinute() {
        #expect(RemoteRdpReconnectPolicy().deadline == 60)
    }
}
