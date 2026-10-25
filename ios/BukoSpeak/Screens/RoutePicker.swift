import AVKit
import SwiftUI

/// The system's picker for where the call plays: the iPhone, its speaker, a headset or Bluetooth.
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
