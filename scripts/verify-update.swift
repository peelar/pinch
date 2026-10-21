// Verify against Pinch's embedded public key, including when CI has a wrong secret.
import Foundation
import CryptoKit
import Darwin

func fail(_ message: String) -> Never {
    fputs("pinch: \(message)\n", stderr)
    exit(1)
}

let args = CommandLine.arguments
guard args.count == 4 else { fail("expected public-key file, archive, and signature") }
do {
    let keyText = try String(contentsOfFile: args[1], encoding: .utf8)
        .trimmingCharacters(in: .whitespacesAndNewlines)
    guard let key = Data(base64Encoded: keyText), let signature = Data(base64Encoded: args[3]) else {
        fail("invalid update public key or signature")
    }
    let publicKey = try Curve25519.Signing.PublicKey(rawRepresentation: key)
    let archive = try Data(contentsOf: URL(fileURLWithPath: args[2]), options: .mappedIfSafe)
    guard publicKey.isValidSignature(signature, for: archive) else {
        fail("update signature does not match Pinch's embedded public key")
    }
} catch {
    fail(error.localizedDescription)
}
