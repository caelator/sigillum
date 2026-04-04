import Foundation
import LocalAuthentication
import Security

enum HelperExit: Int32 {
    case success = 0
    case userCanceled = 1
    case biometryLockedOut = 2
    case helperInternalError = 3
    case keychainAccessDenied = 4
}

struct HelperPayload {
    let version: UInt8 = 1
    let proof: Data
    let keyEncoding: UInt8 = 1
    let key: Data

    func encode() -> Data {
        var out = Data([version])
        out.append(UInt16(proof.count).bigEndianData)
        out.append(proof)
        out.append(Data([keyEncoding]))
        out.append(UInt16(key.count).bigEndianData)
        out.append(key)
        return out
    }
}

extension UInt16 {
    var bigEndianData: Data {
        var value = self.bigEndian
        return Data(bytes: &value, count: MemoryLayout<UInt16>.size)
    }
}

func readNonce() -> Data? {
    let input = FileHandle.standardInput.readDataToEndOfFile()
    return input.count == 32 ? input : nil
}

func mapLAError(_ error: Error) -> HelperExit {
    let nsError = error as NSError
    guard nsError.domain == LAError.errorDomain,
          let code = LAError.Code(rawValue: nsError.code) else {
        return .helperInternalError
    }
    switch code {
    case .userCancel, .appCancel, .systemCancel:
        return .userCanceled
    case .biometryLockout:
        return .biometryLockedOut
    default:
        return .helperInternalError
    }
}

func fetchBiometricKey(context: LAContext) throws -> Data {
    let query: [String: Any] = [
        kSecClass as String: kSecClassGenericPassword,
        kSecAttrAccount as String: "sigillum.biometric.vault-key",
        kSecUseAuthenticationContext as String: context,
        kSecReturnData as String: true,
        kSecMatchLimit as String: kSecMatchLimitOne
    ]
    var result: CFTypeRef?
    let status = SecItemCopyMatching(query as CFDictionary, &result)
    guard status == errSecSuccess, let data = result as? Data else {
        throw NSError(domain: NSOSStatusErrorDomain, code: Int(status))
    }
    return data
}

func signNonce(context: LAContext, nonce: Data) throws -> Data {
    let attributes: [String: Any] = [
        kSecAttrKeyType as String: kSecAttrKeyTypeECSECPrimeRandom,
        kSecAttrKeyClass as String: kSecAttrKeyClassPrivate,
        kSecAttrApplicationTag as String: "sigillum.biometric.signing-key".data(using: .utf8)!,
        kSecUseAuthenticationContext as String: context,
        kSecReturnRef as String: true
    ]
    var item: CFTypeRef?
    let status = SecItemCopyMatching(attributes as CFDictionary, &item)
    guard status == errSecSuccess, let privateKey = item else {
        throw NSError(domain: NSOSStatusErrorDomain, code: Int(status))
    }

    var error: Unmanaged<CFError>?
    guard let signature = SecKeyCreateSignature(
        privateKey as! SecKey,
        .ecdsaSignatureMessageX962SHA256,
        nonce as CFData,
        &error
    ) as Data? else {
        throw error!.takeRetainedValue() as Error
    }
    return signature
}

func main() -> HelperExit {
    guard let nonce = readNonce() else {
        return .helperInternalError
    }

    let context = LAContext()
    context.localizedReason = "Unlock Sigillum"

    var canEvaluateError: NSError?
    guard context.canEvaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, error: &canEvaluateError) else {
        if let error = canEvaluateError {
            return mapLAError(error)
        }
        return .helperInternalError
    }

    let semaphore = DispatchSemaphore(value: 0)
    var authError: Error?
    context.evaluatePolicy(.deviceOwnerAuthenticationWithBiometrics, localizedReason: "Unlock Sigillum") { success, error in
        if !success {
            authError = error
        }
        semaphore.signal()
    }
    semaphore.wait()

    if let error = authError {
        return mapLAError(error)
    }

    do {
        let proof = try signNonce(context: context, nonce: nonce)
        let key = try fetchBiometricKey(context: context)
        let payload = HelperPayload(proof: proof, key: key).encode()
        FileHandle.standardOutput.write(payload)
        return .success
    } catch let error as NSError {
        if error.domain == NSOSStatusErrorDomain {
            return .keychainAccessDenied
        }
        return mapLAError(error)
    } catch {
        return .helperInternalError
    }
}

exit(main().rawValue)
