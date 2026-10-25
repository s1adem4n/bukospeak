import Foundation
import Security

/// Secrets of this app, as generic passwords in the Keychain, readable once the iPhone was
/// unlocked after starting.
enum Keychain {
    struct Failure: Error {
        let status: OSStatus
    }

    private static let service = "digital.buko.BukoSpeak"

    static func get(_ account: String) -> String? {
        var query = query(account)
        query[kSecReturnData] = true
        var data: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &data) == errSecSuccess, let data = data as? Data else {
            return nil
        }
        return String(data: data, encoding: .utf8)
    }

    /// Stores the value, or removes it if it is empty.
    static func set(_ value: String, for account: String) throws(Failure) {
        let deleted = SecItemDelete(query(account) as CFDictionary)
        guard deleted == errSecSuccess || deleted == errSecItemNotFound else { throw Failure(status: deleted) }
        guard !value.isEmpty else { return }
        var item = query(account)
        item[kSecValueData] = Data(value.utf8)
        item[kSecAttrAccessible] = kSecAttrAccessibleAfterFirstUnlock
        let added = SecItemAdd(item as CFDictionary, nil)
        guard added == errSecSuccess else { throw Failure(status: added) }
    }

    private static func query(_ account: String) -> [CFString: Any] {
        [kSecClass: kSecClassGenericPassword, kSecAttrService: service, kSecAttrAccount: account]
    }
}
