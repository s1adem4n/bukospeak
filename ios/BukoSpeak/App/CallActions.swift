import SwiftUI

struct CallActions {
    var join: () -> Void = {}
    var leave: () -> Void = {}
    var toggleMute: () -> Void = {}
    var watch: (Bool) -> Void = { _ in }
}

extension EnvironmentValues {
    @Entry var call = CallActions()
}
