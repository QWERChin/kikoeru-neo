import XCTest

// "Add to Home Screen", then the same lock screen check in the standalone
// web app. It keeps its own storage, so it signs in again.
extension Driver {
    func homeScreenApp() throws {
        foreground(safari)
        if isPlaying { web.buttons["暂停"].firstMatch.tap() }
        try addToHomeScreen()
        home()
        // A new icon lands on the first home screen page with room, past
        // the default apps.
        let icon = springboard.icons["kikoeru"]
        for _ in 0..<4 where !(icon.exists && icon.isHittable) {
            springboard.swipeLeft()
            pause(1)
        }
        guard icon.waitForExistence(timeout: 10), icon.isHittable else {
            screenshot("no-icon")
            dump(springboard, "no-icon")
            throw CheckError("no kikoeru icon on the home screen")
        }
        check("主屏幕上出现 kikoeru 图标", true)
        screenshot("home-screen-icon")
        icon.tap()
        pause(4)
        screenshot("web-app-launched")
        guard let app = findWebApp() else {
            dump(springboard, "web-app-launched")
            throw CheckError("the web app is not one of \(webAppCandidates)")
        }
        webApp = app
        note("主屏幕应用的进程", "\(app)")
        check("从图标打开的是独立窗口，不是 Safari", safari.state != .runningForeground)
        let user = web.textFields.firstMatch
        if user.waitForExistence(timeout: 15) {
            dismissPrompts()
            try enter(settings.user, into: user)
            try enter(settings.password + "\n", into: web.secureTextFields.firstMatch)
            pause(2)
            dismissPrompts()
        }
        check("主屏幕应用登录后看到作品", containing(web.links, "RJ01000001").waitForExistence(timeout: 20))
        screenshot("web-app-library")
        try play("01 long tone")
        pause(3)
        lock()
        let first = trackID("01 long tone.mp3")
        // The page reports every 15 s while it plays.
        let saved = waitForProgress(35) { $0.trackID == first && $0.positionMs >= 14000 }
        check("主屏幕应用锁屏后继续播放", saved != nil, "server: \(lastProgressText)")
        wake()
        screenshot("web-app-lock-screen")
        if containing(springboard.staticTexts, "long tone").exists {
            check("锁屏界面显示主屏幕应用正在放的轨道", true)
        } else {
            skip("主屏幕应用的锁屏正在播放卡片", "读不到，看 web-app-lock-screen 的截图")
        }
        unlock()
        foreground(app)
        screenshot("web-app-unlocked")
        if isPlaying { web.buttons["暂停"].firstMatch.tap() }
    }

    var webAppCandidates: [String] {
        settings.webAppIDs + ["com.apple.webapp", "com.apple.webapp1", "com.apple.WebSheet"]
    }

    private func findWebApp() -> XCUIApplication? {
        for id in webAppCandidates {
            let app = XCUIApplication(bundleIdentifier: id)
            if app.state == .runningForeground { return app }
        }
        return nil
    }

    private func addToHomeScreen() throws {
        var path: [String] = []
        if let share = tapAny(in: safari, ["ShareButton", "Share"], timeout: 2) {
            path.append(share)
        } else if let more = tapAny(in: safari, ["MoreMenuButton", "More"], timeout: 3) {
            path.append(more)
            pause(1)
            screenshot("safari-menu")
            if any(in: safari, ["Add to Home Screen"], timeout: 1) == nil,
               let share = tapAny(in: safari, ["Share", "ShareButton"], timeout: 3) {
                path.append(share)
            }
        } else {
            screenshot("no-share-button")
            dump(safari, "no-share-button")
            throw CheckError("Safari shows neither a Share nor a More button")
        }
        pause(1.5)
        screenshot("share-sheet")
        var add = any(in: safari, ["Add to Home Screen"], timeout: 5)
        if add == nil {
            safari.swipeUp()
            add = any(in: safari, ["Add to Home Screen"], timeout: 3)
        }
        guard let add else {
            dump(safari, "share-sheet")
            throw CheckError("no Add to Home Screen after \(path)")
        }
        add.tap()
        pause(2)
        screenshot("add-to-home-screen")
        dump(safari, "add-to-home-screen")
        let standalone = safari.switches["Open as Web App"]
        if standalone.exists {
            note("“Open as Web App”开关", "value=\(String(describing: standalone.value))")
            if standalone.value as? String == "0" { standalone.tap() }
        }
        guard tapAny(in: safari, ["Add"], timeout: 5) != nil else {
            throw CheckError("no Add button on the Add to Home Screen sheet")
        }
        check("添加到主屏幕", true, path.joined(separator: " > "))
        pause(3)
    }
}
