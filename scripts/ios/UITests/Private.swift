import Foundation

// A private system interface, reached through dlsym so that a missing
// symbol in some runtime only turns the check into "unknown" instead of
// failing the build. MediaRemote was tried for the headset path and dropped:
// the simulator routes a test runner's commands to the Music app, not to the
// page that is playing.

/// SpringBoard's lock state, read the way WebDriverAgent reads it.
enum LockState {
    static func isLocked() -> Bool? {
        typealias RegisterCheck = @convention(c) (UnsafePointer<CChar>, UnsafeMutablePointer<Int32>) -> UInt32
        typealias GetState = @convention(c) (Int32, UnsafeMutablePointer<UInt64>) -> UInt32
        let anywhere = UnsafeMutableRawPointer(bitPattern: -2)
        guard let register = dlsym(anywhere, "notify_register_check"),
              let getState = dlsym(anywhere, "notify_get_state") else { return nil }
        var token: Int32 = 0
        let registered = "com.apple.springboard.lockstate".withCString {
            unsafeBitCast(register, to: RegisterCheck.self)($0, &token)
        }
        guard registered == 0 else { return nil }
        var state: UInt64 = 0
        guard unsafeBitCast(getState, to: GetState.self)(token, &state) == 0 else { return nil }
        return state != 0
    }
}
