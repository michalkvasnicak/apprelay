import SwiftUI

@main
struct HelloWorldApp: App {
    var body: some Scene {
        WindowGroup {
            VStack(spacing: 16) {
                Image(systemName: "hand.wave.fill")
                    .font(.system(size: 64))
                    .foregroundStyle(.blue)
                Text("Hello, World!")
                    .font(.largeTitle.bold())
                Text("Your example iOS app is working.")
                    .foregroundStyle(.secondary)
            }
            .padding()
        }
    }
}
