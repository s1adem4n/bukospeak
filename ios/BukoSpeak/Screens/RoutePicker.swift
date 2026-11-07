import AVKit
import SwiftUI

struct RoutePicker: UIViewRepresentable {
    func makeUIView(context: Context) -> AVRoutePickerView {
        let picker = AVRoutePickerView()
        picker.prioritizesVideoDevices = false
        picker.tintColor = .label
        picker.activeTintColor = .systemGreen
        return picker
    }

    func updateUIView(_ picker: AVRoutePickerView, context: Context) {}
}
