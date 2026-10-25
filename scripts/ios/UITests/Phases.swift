import XCTest

// The phases of the check, in the order the test methods run them. Each one
// leaves the phone in the state the next one starts from, and the timings
// follow the fixture in make-library.sh.
extension Driver {
    func login() throws {
        let user = web.textFields.firstMatch
        // Safari's first launch can come up on its start page and drop the
        // URL it was opened with, so the URL is opened again until the page shows.
        for attempt in 1...3 where !user.exists {
            open("/library")
            if user.waitForExistence(timeout: 15) { break }
            screenshot("open-attempt-\(attempt)")
            note("第 \(attempt) 次打开网址后没看到登录页", tapAny(in: safari, ["Continue", "Not Now", "OK", "Done", "Close", "Get Started"]) ?? "nothing to dismiss")
        }
        guard user.waitForExistence(timeout: 5) else {
            screenshot("no-login-form")
            dump(safari, "no-login-form")
            throw CheckError("the login form did not appear")
        }
        check("HTTPS 证书被信任，页面正常打开", !containing(safari.staticTexts, "Not Private").exists)
        screenshot("login")
        dismissPrompts()
        try enter(settings.user, into: user)
        try enter(settings.password + "\n", into: web.secureTextFields.firstMatch)
        pause(2)
        dismissPrompts()
        let library = containing(web.links, "RJ01000001")
        check("登录后看到资料库里的作品", library.waitForExistence(timeout: 20))
        dismissPrompts()
        screenshot("library")
        library.tap()
        check("打开作品详情和曲目列表", trackButton("01 long tone").waitForExistence(timeout: 15))
        screenshot("work")
    }

    func playAndSeek() throws {
        _ = api.clearProgress(work: settings.workID)
        try play("01 long tone")
        check("点曲目后开始播放", true)
        pause(3)
        screenshot("work-playing")
        try openPlayerPage()
        guard let moving = advancing() else { throw CheckError("no clock on the player page") }
        check("播放时间在走", moving.1 > moving.0, "\(moving.0) s -> \(moving.1) s")
        web.buttons["前进 30 秒"].firstMatch.tap()
        pause(1.5)
        let after = clock()
        check("前进 30 秒", (after?.position ?? 0) >= moving.1 + 25, "\(moving.1) s -> \(after?.position ?? -1) s")
        screenshot("player-after-seek")
    }

    func background() throws {
        try openPlayerPage()
        guard isPlaying, let before = clock() else { throw CheckError("not playing before going home") }
        home()
        screenshot("home-screen")
        pause(10)
        foreground(safari)
        let after = clock()
        let detail = "\(before.position)/\(before.duration) s -> \(after.map { "\($0.position)/\($0.duration)" } ?? "?") s"
        if let after, after.duration != before.duration {
            check("在桌面时这一轨放完，自动换到下一轨接着放", isPlaying, detail)
        } else {
            check("回到桌面约 12 秒后，位置前进了", (after?.position ?? 0) - before.position >= 10, detail)
        }
        screenshot("back-from-home")
    }

    /// The top folder's tracks in queue order, told apart by duration.
    private static let queue: [(seconds: Int, title: String, path: String)] = [
        (90, "long tone", "01 long tone.mp3"),
        (50, "middle tone", "02 middle tone.mp3"),
        (30, "short tone", "03 short tone.mp3"),
        (20, "transcode", "04 transcode.wma"),
    ]

    func lockAndAutoNext() throws {
        try openPlayerPage()
        guard isPlaying, var start = clock() else { throw CheckError("not playing before locking") }
        // The earlier phases normally leave the first track under 30 s from
        // its end. On a slow run it may have moved on, so the check starts
        // from whichever track is playing.
        if start.duration - start.position >= 30 {
            web.buttons["前进 30 秒"].firstMatch.tap()
            pause(1.5)
            start = clock() ?? start
        }
        guard let index = Self.queue.firstIndex(where: { $0.seconds == start.duration }),
              index + 2 < Self.queue.count else {
            throw CheckError("cannot tell which track is playing from \(start)")
        }
        let next = Self.queue[index + 1]
        let remaining = max(0, start.duration - start.position)
        note("锁屏前", "\(Self.queue[index].title) at \(start.position)/\(start.duration) s")
        lock()
        check("锁屏", LockState.isLocked() == true, "lockstate=\(String(describing: LockState.isLocked()))")
        // A pause also saves the next track at 0 s, so only a position the
        // 15 s report sends while it plays shows it really played locked.
        let nextID = trackID(next.path)
        let seen = waitForProgress(Double(remaining) + 30) { $0.trackID == nextID && $0.positionMs >= 10_000 }
        check("锁屏时这一轨放完，自动接着放下一轨（\(next.title)）", seen != nil, "server: \(lastProgressText)")
        wake()
        screenshot("lock-screen")
        dump(springboard, "lock-screen")
        var shown = containing(springboard.staticTexts, next.title).exists
        if !shown {
            // The simulator draws Now Playing only from the second lock on.
            lock()
            pause(3)
            wake()
            screenshot("lock-screen-again")
            shown = containing(springboard.staticTexts, next.title).exists
        }
        if shown {
            check("锁屏界面显示正在播放的轨道", true)
            lockScreenControls()
        } else {
            // iOS 26 draws the card as a Live Activity outside SpringBoard's
            // accessibility tree, so only the screenshot shows it.
            skip("锁屏界面的正在播放卡片", "读不到，看 lock-screen-again 的截图：\(labels(springboard, .staticText))")
        }
        unlock()
        foreground(safari)
        screenshot("unlocked")
        let beforeCenter = clock()
        let skipped = controlCenter()
        foreground(safari)
        let now = clock()
        check("解锁后页面仍在播放", isPlaying, "\(String(describing: now))")
        if skipped {
            check("控制中心的下一首让页面换了轨道", now?.duration != beforeCenter?.duration, "\(String(describing: beforeCenter)) -> \(String(describing: now))")
        }
    }

    /// Pause, play and next on whichever system surface is showing. True
    /// when next was tapped.
    @discardableResult
    private func systemControls(_ place: String) -> Bool {
        if tapAny(in: springboard, ["Pause", "暂停"]) != nil {
            check("\(place)的暂停按钮", any(in: springboard, ["Play", "播放"], timeout: 5) != nil)
            screenshot("\(place)-paused")
            if tapAny(in: springboard, ["Play", "播放"]) != nil {
                check("\(place)的播放按钮", any(in: springboard, ["Pause", "暂停"], timeout: 5) != nil)
            }
        } else {
            check("\(place)有暂停按钮", false, labels(springboard, .button))
        }
        if let tapped = tapAny(in: springboard, ["Next Track", "Next", "下一首", "下一曲"]) {
            pause(2)
            screenshot("\(place)-next")
            note("\(place)点了下一首", tapped)
            return true
        }
        check("\(place)有下一首按钮", false, labels(springboard, .button))
        return false
    }

    private func lockScreenControls() {
        systemControls("锁屏界面")
    }

    /// Control Center's Now Playing module drives the page through the same
    /// remote commands as the lock screen and a headset's buttons.
    private func controlCenter() -> Bool {
        let top = springboard.coordinate(withNormalizedOffset: CGVector(dx: 0.92, dy: 0.005))
        top.press(forDuration: 0.1, thenDragTo: springboard.coordinate(withNormalizedOffset: CGVector(dx: 0.92, dy: 0.6)))
        pause(2)
        screenshot("control-center")
        dump(springboard, "control-center")
        let titles = Self.queue.map(\.title)
        let shown = titles.contains { containing(springboard.staticTexts, $0).exists }
        if shown {
            check("控制中心显示正在播放的轨道", true)
        } else {
            skip("控制中心的正在播放模块", "模拟器的控制中心是空的：\(labels(springboard, .staticText))")
        }
        let skipped = shown && systemControls("控制中心")
        XCUIDevice.shared.press(.home)
        pause(1.5)
        return skipped
    }

    func refreshRestore() throws {
        try play("02 middle tone")
        pause(6)
        web.buttons["暂停"].firstMatch.tap()
        pause(2)
        let second = trackID("02 middle tone.mp3")
        guard let saved = waitForProgress(10, { $0.trackID == second && $0.positionMs > 3000 }) else {
            throw CheckError("the pause did not save a position on track 2: \(lastProgressText)")
        }
        check("暂停时服务器保存了位置", true, "\(saved)")
        let method = reload()
        note("刷新页面的方式", method)
        pause(3)
        screenshot("after-reload")
        try openWorkPage()
        let resume = any(in: browser, ["继续播放", "已暂停", "播放"], timeout: 10)
        guard let resume else { throw CheckError("no resume button after the reload") }
        let hint = containing(web.staticTexts, "上次听到")
        note("刷新后作品页的提示", hint.exists ? hint.label : "(no hint, button \(resume.label))")
        resume.tap()
        guard wait(15, until: { self.isPlaying }) else { throw CheckError("did not play after the reload") }
        try openPlayerPage()
        let at = clock()
        let savedSeconds = saved.positionMs / 1000
        let position = at?.position ?? -1
        // 02 middle tone is the only 50 s track, so the duration names it.
        check("刷新后从保存的那一轨和位置继续", at?.duration == 50 && position >= savedSeconds - 1 && position <= savedSeconds + 8, "saved \(savedSeconds) s on 02 middle tone, now \(String(describing: at))")
        screenshot("resumed")
    }

    /// Reloads the tab the way a person would: the reload button if the
    /// toolbar shows one, otherwise pull to refresh.
    private func reload() -> String {
        if let tapped = tapAny(in: safari, ["ReloadButton"], timeout: 2) {
            return "button \(tapped)"
        }
        web.swipeDown()
        web.swipeDown()
        return "pull to refresh"
    }

    /// The server becomes unreachable for 20 s. The track that is playing
    /// should go on from what it already loaded. The next one cannot load
    /// until the server is back.
    func outage() throws {
        try openPlayerPage()
        guard isPlaying else { throw CheckError("not playing before the outage") }
        let response = control.call("GET", "/down?seconds=20")
        check("让服务器 20 秒连不上", response.status == 200, "status \(response.status)")
        pause(4)
        let moving = advancing(over: 3)
        check("连不上时正在放的这一轨继续播放", isPlaying && (moving.map { $0.1 > $0.0 } ?? false), "\(String(describing: moving))")
        web.buttons["下一轨"].firstMatch.tap()
        pause(5)
        screenshot("outage")
        note("连不上时切到下一轨", isPlaying ? "playing" : "not playing: \(labels(browser, .staticText).prefix(300))")
        pause(10)
        let recovered = wait(25) {
            guard self.isPlaying, let moving = self.advancing(over: 2) else { return false }
            return moving.1 > moving.0
        }
        // The player retries by itself once the network is back
        // (book/src/media.md), so not resuming is a failure. Whether 重试
        // helps is only recorded, to tell a stuck player from a dead one.
        check("恢复连接后自己接着播放", recovered)
        if !recovered {
            screenshot("outage-stuck")
            let retried = tapAny(in: browser, ["重试"], timeout: 2) != nil
            let resumed = wait(15) {
                guard self.isPlaying, let moving = self.advancing(over: 2) else { return false }
                return moving.1 > moving.0
            }
            note("点“重试”后恢复播放", retried ? (resumed ? "resumed" : "still stopped") : "no 重试 button")
        }
        screenshot("after-outage")
    }

    func transcode() throws {
        try play("04 transcode", within: 60)
        try openPlayerPage()
        let moving = advancing()
        check("wma 转码后能播放", (moving?.1 ?? 0) > (moving?.0 ?? 0), "\(String(describing: moving))")
        screenshot("transcoded")
        web.buttons["暂停"].firstMatch.tap()
    }

    func video() throws {
        try play("01 picture")
        pause(2)
        screenshot("video-on-work-page")
        try openPlayerPage()
        screenshot("video-player")
        let fullscreen = web.buttons["全屏"].firstMatch
        if fullscreen.waitForExistence(timeout: 5) {
            fullscreen.tap()
            pause(3)
            screenshot("fullscreen")
            note("全屏时的按钮", labels(safari, .button))
            check("进入全屏", web.buttons["退出全屏"].exists || !fullscreen.isHittable)
            XCUIDevice.shared.orientation = .landscapeLeft
            pause(3)
            screenshot("fullscreen-landscape")
            XCUIDevice.shared.orientation = .portrait
            pause(2)
            // A tap in the middle of the system player can land on its play
            // button, so the controls are woken near the top instead.
            var closed = tapAny(in: safari, Self.exitFullscreen, timeout: 2)
            if closed == nil {
                safari.coordinate(withNormalizedOffset: CGVector(dx: 0.5, dy: 0.15)).tap()
                closed = tapAny(in: safari, Self.exitFullscreen, timeout: 3)
            }
            if closed == nil { safari.swipeDown() }
            pause(2)
            check("退出全屏回到播放页", web.sliders["播放进度"].waitForExistence(timeout: 5) && fullscreen.isHittable, closed ?? "swipe down")
            screenshot("after-fullscreen")
            // The system player's close button tells WebKit the user is done
            // with the media, and WebKit pauses it (2026-10-15, iOS 26.5).
            note("退出全屏后视频", isPlaying ? "接着播放" : "暂停了（\(closed ?? "swipe down")）")
        } else {
            check("播放页有全屏按钮", false)
        }
        // aria-pressed makes it a toggle to accessibility, not a button.
        let pip = web.descendants(matching: .any)["画中画"].firstMatch
        guard pip.waitForExistence(timeout: 5) else {
            check("播放页有画中画按钮", false, labels(safari, .button))
            return
        }
        if !isPlaying { web.buttons["播放"].firstMatch.tap() }
        pip.tap()
        pause(3)
        screenshot("pip-in-safari")
        // The page saves its position as it is hidden, so the baseline is
        // the playhead at that moment, not whatever the server held before.
        let video = trackID("video/01 picture.mp4")
        let baseline = (clock()?.position ?? 0) * 1000
        home()
        pause(2)
        // Whether the small window appeared is only in the screenshot. The
        // simulator has been seen to refuse picture in picture (WebKit logs
        // failedToEnterPictureInPicture), and the video then plays on
        // without it.
        screenshot("pip-on-home-screen")
        dump(springboard, "pip-on-home-screen")
        let later = waitForProgress(25) { $0.trackID == video && $0.positionMs >= baseline + 8000 }
        check("点画中画后回到桌面，视频继续播放", later != nil, "from \(baseline) ms, server: \(lastProgressText)")
        foreground(safari)
        let leave = web.descendants(matching: .any)["退出画中画"].firstMatch
        if leave.waitForExistence(timeout: 3) { leave.tap() }
        pause(2)
        screenshot("after-pip")
        if isPlaying { web.buttons["暂停"].firstMatch.tap() }
    }

    func hls() throws {
        try play("02 picture", within: 60)
        try openPlayerPage()
        let moving = advancing()
        check("mkv 走 HLS 能播放", (moving?.1 ?? 0) > (moving?.0 ?? 0), "\(String(describing: moving))")
        screenshot("hls")
        web.buttons["暂停"].firstMatch.tap()
    }
}
