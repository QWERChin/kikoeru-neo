import SwiftUI

// The UI test target needs an app to be installed with it. The checks never
// launch this one; they drive Safari and SpringBoard.
@main
struct HostApp: App {
    var body: some Scene {
        WindowGroup {
            Text("kikoeru-neo iOS check")
        }
    }
}
