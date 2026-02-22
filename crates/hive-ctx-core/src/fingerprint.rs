use blake3::Hasher;

pub fn fingerprint_bytes(bytes: &[u8]) -> [u8; 32] {
  let mut hasher = Hasher::new();
  hasher.update(bytes);
  *hasher.finalize().as_bytes()
}

