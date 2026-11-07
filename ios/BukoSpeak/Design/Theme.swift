import SwiftUI

enum Motion {
    static let spring = Animation.spring(response: 0.35, dampingFraction: 0.85)
    static let speaking = Animation.easeOut(duration: 0.15)
}
