import XCTest

/// One test method per phase, run in name order. A phase that throws is
/// recorded as failed and the next one still runs, so one bad step does not
/// cost the whole macOS run.
final class SafariChecks: XCTestCase {
    static var driver: Driver?

    override func setUp() {
        continueAfterFailure = true
        executionTimeAllowance = 300
    }

    @MainActor
    private func phase(_ name: String, _ body: (Driver) throws -> Void) {
        let driver: Driver
        do {
            if Self.driver == nil { Self.driver = try Driver(settings: try Settings.fromEnvironment()) }
            guard let existing = Self.driver else { return }
            driver = existing
        } catch {
            XCTFail("setup: \(error)")
            return
        }
        driver.phase = name
        do {
            try body(driver)
        } catch {
            driver.fail("这一步没能做完", error)
            driver.screenshot("failed")
            driver.dump(driver.browser, "failed")
        }
    }

    @MainActor func test01Login() { phase("login") { try $0.login() } }
    @MainActor func test02PlayAndSeek() { phase("play") { try $0.playAndSeek() } }
    @MainActor func test03Background() { phase("background") { try $0.background() } }
    @MainActor func test04LockScreen() { phase("lock") { try $0.lockAndAutoNext() } }
    @MainActor func test05RefreshRestore() { phase("restore") { try $0.refreshRestore() } }
    @MainActor func test06Outage() { phase("outage") { try $0.outage() } }
    @MainActor func test07Transcode() { phase("transcode") { try $0.transcode() } }
    @MainActor func test08Video() { phase("video") { try $0.video() } }
    @MainActor func test09HLS() { phase("hls") { try $0.hls() } }
    @MainActor func test10HomeScreenApp() { phase("pwa") { try $0.homeScreenApp() } }
}
