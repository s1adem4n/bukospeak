import SwiftUI

/// How things move.
enum Motion {
    /// Layout changes: members coming and going, phases changing.
    static let spring = Animation.spring(response: 0.35, dampingFraction: 0.85)
    /// The speaking ring, quick enough to follow speech.
    static let speaking = Animation.easeOut(duration: 0.15)
}
