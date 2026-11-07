import SwiftUI

enum Space {
    static let line: CGFloat = 4
    static let tight: CGFloat = 6
    static let inline: CGFloat = 8
    static let gap: CGFloat = 12
    static let gutter: CGFloat = 16
    static let section: CGFloat = 24
}

enum Measure {
    static let hairline: CGFloat = 1
    static let control: CGFloat = 56
    // The route picker draws its glyph as large as itself.
    static let routePicker: CGFloat = 40
    static let field: CGFloat = 52
    static let value: CGFloat = 44
    static let dot: CGFloat = 7
    static let dotHalo: CGFloat = 3
    static let speakingEdge: CGFloat = 2
    static let mark: CGFloat = 80
    static let welcome: CGFloat = 320
}

enum Radius {
    static let field: CGFloat = 16
}

// The desktop's springs, so a tile lights and dims alike in both apps.
enum Motion {
    static let spring = Animation.spring(response: 0.35, dampingFraction: 0.85)
    static let speakOn = Animation.interpolatingSpring(mass: 1, stiffness: 900, damping: 60)
    static let speakOff = Animation.interpolatingSpring(mass: 1, stiffness: 110, damping: 21)
    static let dim = Animation.interpolatingSpring(mass: 1, stiffness: 440, damping: 42)
    static let waiting = Animation.easeInOut(duration: 0.7).repeatForever()
}

enum Dim {
    static let reconnecting = 0.45
    static let waiting = 0.4
}
