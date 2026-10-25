import XCTest

/// Drives Mobile Safari, a home screen web app and SpringBoard like a person
/// holding the phone, and keeps the report. Checks record a result and carry
/// on; a CheckError ends only the current phase.
@MainActor
final class Driver {
    let settings: Settings
    let report: Report
    let api: API
    let control: API
    let safari = XCUIApplication(bundleIdentifier: "com.apple.mobilesafari")
    let springboard = XCUIApplication(bundleIdentifier: "com.apple.springboard")
    /// The home screen web app, once the last phase opens it.
    var webApp: XCUIApplication?
    var trackIDs: [String: Int] = [:]
    /// What the server held at the end of the last waitForProgress.
    var lastProgress: API.Progress?
    var phase = "setup"
    private var artifactCount = 0

    init(settings: Settings) throws {
        self.settings = settings
        report = Report(outDir: settings.outDir)
        api = API(base: settings.baseURL)
        control = API(base: settings.controlURL)
        guard api.login(user: settings.user, password: settings.password) else {
            throw CheckError("the test could not log in to the API as \(settings.user)")
        }
        trackIDs = api.tracks(work: settings.workID)
        record("setup", "API 登录并读到曲目", trackIDs.count == 6 ? "pass" : "fail", "\(trackIDs.count) tracks")
    }

    /// Safari, or the home screen web app when one is open.
    var browser: XCUIApplication { webApp ?? safari }
    var web: XCUIElement { browser.webViews.firstMatch }

    // MARK: Results

    private func record(_ check: String, _ result: String, _ detail: String) {
        report.record(phase, check, result, detail)
        if result == "fail" { XCTFail("\(phase): \(check): \(detail)") }
    }

    private func record(_ phase: String, _ check: String, _ result: String, _ detail: String) {
        report.record(phase, check, result, detail)
        if result == "fail" { XCTFail("\(phase): \(check): \(detail)") }
    }

    @discardableResult
    func check(_ name: String, _ passed: Bool, _ detail: String = "") -> Bool {
        record(name, passed ? "pass" : "fail", detail)
        return passed
    }

    func note(_ name: String, _ detail: String) {
        record(name, "note", detail)
    }

    /// Something the simulator cannot show, as opposed to a failure.
    func skip(_ name: String, _ detail: String) {
        record(name, "skip", detail)
    }

    func fail(_ name: String, _ error: Error) {
        record(name, "fail", "\(error)")
    }

    // MARK: Evidence

    func screenshot(_ name: String) {
        artifactCount += 1
        let label = String(format: "%02d-%@-%@", artifactCount, phase, name)
        let shot = XCUIScreen.main.screenshot()
        XCTContext.runActivity(named: "screenshot \(label)") { activity in
            let attachment = XCTAttachment(screenshot: shot)
            attachment.name = label
            attachment.lifetime = .keepAlways
            activity.add(attachment)
        }
        if let dir = settings.outDir {
            try? shot.pngRepresentation.write(to: dir.appendingPathComponent("\(label).png"))
        }
    }

    /// The accessibility tree, for working out labels after a failed run.
    func dump(_ app: XCUIApplication, _ name: String) {
        artifactCount += 1
        let label = String(format: "%02d-%@-%@", artifactCount, phase, name)
        let tree = app.debugDescription
        XCTContext.runActivity(named: "tree \(label)") { activity in
            let attachment = XCTAttachment(string: tree)
            attachment.name = label
            attachment.lifetime = .keepAlways
            activity.add(attachment)
        }
        if let dir = settings.outDir {
            try? tree.write(to: dir.appendingPathComponent("\(label).txt"), atomically: true, encoding: .utf8)
        }
    }

    // MARK: Waiting

    func pause(_ seconds: TimeInterval) {
        RunLoop.current.run(until: Date(timeIntervalSinceNow: seconds))
    }

    func wait(_ timeout: TimeInterval, every interval: TimeInterval = 0.5, until condition: () -> Bool) -> Bool {
        let deadline = Date(timeIntervalSinceNow: timeout)
        while Date() < deadline {
            if condition() { return true }
            pause(interval)
        }
        return condition()
    }

    // MARK: Finding things

    func prefixed(_ query: XCUIElementQuery, _ prefix: String) -> XCUIElement {
        query.matching(NSPredicate(format: "label BEGINSWITH %@", prefix)).firstMatch
    }

    func containing(_ query: XCUIElementQuery, _ text: String) -> XCUIElement {
        query.matching(NSPredicate(format: "label CONTAINS %@", text)).firstMatch
    }

    /// The first element of any common kind whose label or identifier is one
    /// of `names`, waiting up to `timeout` for one to appear.
    func any(in app: XCUIApplication, _ names: [String], timeout: TimeInterval = 3) -> XCUIElement? {
        let predicate = NSPredicate(format: "label IN %@ OR identifier IN %@", argumentArray: [names, names])
        let queries = [app.buttons, app.cells, app.menuItems, app.switches, app.staticTexts, app.links]
        var found: XCUIElement?
        _ = wait(timeout) {
            for query in queries {
                let match = query.matching(predicate).firstMatch
                if match.exists && match.isHittable {
                    found = match
                    return true
                }
            }
            return false
        }
        return found
    }

    @discardableResult
    func tapAny(in app: XCUIApplication, _ names: [String], timeout: TimeInterval = 3) -> String? {
        guard let element = any(in: app, names, timeout: timeout) else { return nil }
        let label = element.label.isEmpty ? element.identifier : element.label
        element.tap()
        return label
    }

    /// Labels and identifiers of one element type, from a single snapshot.
    /// Walking a live query instead fails the test when the screen changes
    /// between reading the count and reading an element.
    func labels(_ app: XCUIApplication, _ type: XCUIElement.ElementType) -> String {
        guard let root = try? app.snapshot() else { return "(no snapshot)" }
        var found: [String] = []
        func walk(_ node: XCUIElementSnapshot) {
            if node.elementType == type && found.count < 80 { found.append("\(node.label)|\(node.identifier)") }
            for child in node.children { walk(child) }
        }
        walk(root)
        return found.joined(separator: ", ")
    }

    /// Sheets and tips Safari or the system may put over the page. A fresh
    /// Safari points at its menu with a tip whose close button swallows the
    /// next tap. The page's own close buttons are labelled 关闭, not Close.
    func dismissPrompts() {
        for app in [browser, springboard] {
            for name in ["Not Now", "Never for This Website", "Don’t Allow", "Don't Allow", "Close"] {
                let button = app.buttons[name].firstMatch
                if button.exists && button.isHittable { button.tap() }
            }
        }
    }

    /// Types into a page field once the software keyboard is up, since a tap
    /// that lands on a tip or toast leaves the field without focus.
    func enter(_ text: String, into field: XCUIElement) throws {
        for _ in 0..<3 {
            field.tap()
            if browser.keyboards.firstMatch.waitForExistence(timeout: 4) {
                field.typeText(text)
                return
            }
            dismissPrompts()
        }
        screenshot("no-keyboard")
        throw CheckError("no keyboard came up for \(field.label)")
    }

    // MARK: Phone

    func home() {
        XCUIDevice.shared.press(.home)
        pause(1.5)
    }

    func lock() {
        XCUIDevice.shared.perform(NSSelectorFromString("pressLockButton"))
        pause(2)
    }

    /// From a locked, dark screen to the lock screen. A second press unlocks.
    func wake() {
        XCUIDevice.shared.press(.home)
        pause(2)
    }

    func unlock() {
        XCUIDevice.shared.press(.home)
        pause(2)
        if LockState.isLocked() == true {
            XCUIDevice.shared.press(.home)
            pause(2)
        }
    }

    func foreground(_ app: XCUIApplication) {
        if app.state != .runningForeground {
            app.activate()
            _ = app.wait(for: .runningForeground, timeout: 10)
        }
        pause(1)
    }

    func open(_ path: String) {
        guard let url = URL(string: path, relativeTo: settings.baseURL)?.absoluteURL else { return }
        XCUIDevice.shared.system.open(url)
        _ = safari.wait(for: .runningForeground, timeout: 15)
        pause(2)
        dismissPrompts()
    }

    /// The buttons that leave the system's fullscreen video player.
    static let exitFullscreen = ["Done", "Close", "Exit Full Screen", "Exit full screen", "完成", "关闭"]

    // MARK: The player as the page shows it

    /// Playing when a pause button is on screen: the mini player's, or the player page's.
    var isPlaying: Bool { web.buttons["暂停"].exists }

    func trackButton(_ name: String) -> XCUIElement {
        prefixed(web.buttons, name)
    }

    func trackID(_ path: String) -> Int? { trackIDs[path] }

    /// The position and duration from the player page's progress slider,
    /// whose accessible value is its `aria-valuetext`, "m:ss / m:ss".
    func clock() -> (position: Int, duration: Int)? {
        let slider = web.sliders["播放进度"]
        guard slider.waitForExistence(timeout: 3) else { return nil }
        let text = slider.value as? String ?? ""
        let parts = text.components(separatedBy: " / ")
        guard parts.count == 2, let position = seconds(parts[0]), let duration = seconds(parts[1]) else {
            note("读不出进度条的值", "value=\(text)")
            return nil
        }
        return (position, duration)
    }

    private func seconds(_ clock: String) -> Int? {
        let fields = clock.trimmingCharacters(in: .whitespaces).split(separator: ":").map { Int($0) }
        guard fields.count >= 2, !fields.contains(where: { $0 == nil }) else { return nil }
        return fields.compactMap { $0 }.reduce(0) { $0 * 60 + $1 }
    }

    /// Two readings `gap` seconds apart, for "is the time moving".
    func advancing(over gap: TimeInterval = 3) -> (Int, Int)? {
        guard let first = clock() else { return nil }
        pause(gap)
        guard let second = clock() else { return nil }
        return (first.position, second.position)
    }

    func openPlayerPage() throws {
        if web.sliders["播放进度"].exists { return }
        let link = prefixed(web.links, "打开播放页")
        guard link.waitForExistence(timeout: 10) else { throw CheckError("no mini player link to the player page") }
        link.tap()
        guard web.sliders["播放进度"].waitForExistence(timeout: 10) else {
            throw CheckError("the player page did not open")
        }
    }

    /// The work page, reached inside the app so no new Safari tab opens.
    func openWorkPage() throws {
        let firstTrack = trackButton("01 long tone")
        if firstTrack.exists { return }
        if !web.exists || !web.isHittable {
            // A video left in fullscreen covers the page.
            browser.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.5)).tap()
            _ = tapAny(in: browser, Self.exitFullscreen, timeout: 2)
        }
        let close = web.buttons["关闭播放页"]
        if close.exists {
            close.tap()
            if firstTrack.waitForExistence(timeout: 5) { return }
        }
        let library = web.links["资料库"]
        if library.waitForExistence(timeout: 5) {
            library.tap()
            let card = containing(web.links, "RJ01000001")
            if card.waitForExistence(timeout: 10) {
                card.tap()
                if firstTrack.waitForExistence(timeout: 10) { return }
            }
        }
        throw CheckError("could not reach the work page")
    }

    func play(_ track: String, within timeout: TimeInterval = 20) throws {
        try openWorkPage()
        let button = trackButton(track)
        if !button.exists {
            let folder = prefixed(web.buttons, "video")
            if folder.exists { folder.tap() }
        }
        guard button.waitForExistence(timeout: 5) else { throw CheckError("no track \(track) on the work page") }
        button.tap()
        guard wait(timeout, until: { self.isPlaying }) else {
            screenshot("not-playing")
            throw CheckError("\(track) did not start playing within \(Int(timeout)) s")
        }
    }

    /// Waits for the server to hold progress that `matches`, polling while
    /// the phone does whatever it is doing.
    /// Nil when nothing matched in time. `lastProgress` keeps the last
    /// answer for the failure message.
    func waitForProgress(_ timeout: TimeInterval, _ matches: (API.Progress) -> Bool) -> API.Progress? {
        var last: API.Progress?
        let matched = wait(timeout, every: 2) {
            last = self.api.progress(work: self.settings.workID)
            return last.map(matches) ?? false
        }
        lastProgress = last
        return matched ? last : nil
    }

    var lastProgressText: String { lastProgress.map(String.init(describing:)) ?? "no answer" }
}
