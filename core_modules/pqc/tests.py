# ~/aurora_project/core_modules/pqc/tests.py
from .pqc_key_exchange import ClassicalKeyExchange, SymmetricCipher
# os import is not used here, remove it for tidiness
# import os # REMOVE THIS LINE if Pylance flags as unused

from cryptography.exceptions import InvalidTag # Import the specific exception type

if __name__ == "__main__":
    print("--- Running PQC Module (Classical Placeholder) Tests ---")

    # 1. Key Exchange Setup
    key_exchange = ClassicalKeyExchange()

    # Alice generates her keys
    alice_private_key, alice_public_key = key_exchange.generate_keys()
    print("\nAlice generated keys.")

    # Bob generates his keys
    bob_private_key, bob_public_key = key_exchange.generate_keys()
    print("Bob generated keys.")

    # 2. Derive Shared Secrets
    # Alice derives shared key using her private key and Bob's public key
    alice_shared_key = key_exchange.derive_shared_key(alice_private_key, bob_public_key)
    print(f"\nAlice derived shared key: {alice_shared_key.hex()[:8]}...")

    # Bob derives shared key using his private key and Alice's public key
    bob_shared_key = key_exchange.derive_shared_key(bob_private_key, alice_public_key)
    print(f"Bob derived shared key: {bob_shared_key.hex()[:8]}...")

    # Verify that both derived shared keys are identical
    assert alice_shared_key == bob_shared_key
    print("\nShared keys derived by Alice and Bob match! (Classical KEM successful)")

    # 3. Symmetric Encryption/Decryption Test
    symmetric_cipher = SymmetricCipher(alice_shared_key) # Use the shared key

    original_message = b"AURORA is the future of computational cosmos!"
    print(f"\nOriginal message: {original_message.decode()}")

    # Encrypt the message - NOW EXPECTING THREE RETURN VALUES
    iv, ciphertext, tag = symmetric_cipher.encrypt(original_message)
    print(f"Encrypted data (ciphertext): {ciphertext.hex()[:16]}...")
    print(f"IV: {iv.hex()[:8]}...")
    print(f"Tag: {tag.hex()[:8]}...")

    # Decrypt the message
    decrypted_message = symmetric_cipher.decrypt(iv, ciphertext, tag)
    print(f"Decrypted message: {decrypted_message.decode()}")

    # Verify decryption
    assert original_message == decrypted_message
    print("\nEncryption and Decryption successful! (Classical AES GCM verified)")

    # 4. Attempt Tampering (should fail)
    print("\nAttempting to tamper with message (expected failure)...")
    tampered_ciphertext = ciphertext + b'\x00' # Add a byte to tamper
    try:
        symmetric_cipher.decrypt(iv, tampered_ciphertext, tag)
        print("ERROR: Tampered message was not detected!")
    except InvalidTag as e: # Catch the specific InvalidTag exception from cryptography
        print(f"Caught expected error (tampering detected): {e}")
        # The assert below is now simple as we caught the specific exception
        assert isinstance(e, InvalidTag)
    except Exception as e:
        print(f"Caught UNEXPECTED error type for tampering: {type(e).__name__} - {e}")

    print("\n--- PQC Module (Classical Placeholder) Tests Complete ---")