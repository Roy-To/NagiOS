use lindera_dictionary::dictionary::{
    character_definition::CharacterDefinition, connection_cost_matrix::ConnectionCostMatrix,
    metadata::Metadata, prefix_dictionary::PrefixDictionary, unknown_dictionary::UnknownDictionary,
    Dictionary,
};
use sha2::{Digest, Sha256};

pub const LOCKED_VOICE_BYTES: usize = 2_154_716;
pub const MAX_DICTIONARY_COMPONENT_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_DICTIONARY_TOTAL_BYTES: usize = 128 * 1024 * 1024;

const VOICE_SHA256: &str = "ded6acb4243b93cc50896199d67b00a7c487662bfcdcd3f9f30a4fb0444b0b46";

const DICTIONARY_SHA256: [&str; 8] = [
    "8fa56e3d131c9d7cabc3633bb7c2add979469a1e22be90ecd714e81df20a5dd4", // metadata.json
    "cd3dbb6fa448906590c2e5ef2600378e4bd074c4d1d6fc5c049d4fd8c2d2d556", // char_def.bin
    "a64b58ac89b2ca21f3ae6c56ca772f8d4488cc636f47b821b4cca9c0c67c95d2", // matrix.mtx
    "26f14fe3affdb0966e1696ae6cd4b7faac91bf85b7ca57a5d8cdc045da58d043", // dict.da
    "7fb308e1909be247779be0a4c81070416635c4ff0ab266d48a5e964baffd345f", // dict.vals
    "67b30a728ab8816def37d9e2775d99dc98bf8eaa6f74a821dd3ee2fd009e8b61", // dict.wordsidx
    "7ef63438285a3dd50299aa826f3d62d9657a907184e055a179af3e00eae820aa", // dict.words
    "1ff6b44ab3145cd8ac731b9cb0500804feae5fa257aee8bc78a65a8dc80bb454", // unk.bin
];

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum AssetError {
    VoiceMismatch,
    DictionaryMismatch,
    DictionaryTooLarge,
    DictionaryDecode,
    VoiceDecode,
    UnsupportedVoice,
}

/// Owned bytes from the existing read-only Model Store asset boundary.
pub struct DictionaryAssets {
    pub metadata_json: Vec<u8>,
    pub char_def_bin: Vec<u8>,
    pub matrix_mtx: Vec<u8>,
    pub dict_da: Vec<u8>,
    pub dict_vals: Vec<u8>,
    pub dict_wordsidx: Vec<u8>,
    pub dict_words: Vec<u8>,
    pub unk_bin: Vec<u8>,
}

impl DictionaryAssets {
    fn components(&self) -> [(&'static str, &[u8]); 8] {
        [
            ("metadata.json", &self.metadata_json),
            ("char_def.bin", &self.char_def_bin),
            ("matrix.mtx", &self.matrix_mtx),
            ("dict.da", &self.dict_da),
            ("dict.vals", &self.dict_vals),
            ("dict.wordsidx", &self.dict_wordsidx),
            ("dict.words", &self.dict_words),
            ("unk.bin", &self.unk_bin),
        ]
    }

    fn lengths(&self) -> [usize; 8] {
        let parts = self.components();
        core::array::from_fn(|i| parts[i].1.len())
    }

    pub(crate) fn verify_and_decode(self) -> Result<Dictionary, AssetError> {
        let parts = self.components();
        let lengths = self.lengths();
        validate_dictionary_lengths(lengths)?;
        for (index, (_, bytes)) in parts.iter().enumerate() {
            if !digest_matches(bytes, DICTIONARY_SHA256[index]) {
                return Err(AssetError::DictionaryMismatch);
            }
        }

        let Self {
            metadata_json,
            char_def_bin,
            matrix_mtx,
            dict_da,
            dict_vals,
            dict_wordsidx,
            dict_words,
            unk_bin,
        } = self;

        let metadata: Metadata =
            serde_json::from_slice(&metadata_json).map_err(|_| AssetError::DictionaryDecode)?;
        let character_definition =
            CharacterDefinition::load(&char_def_bin).map_err(|_| AssetError::DictionaryDecode)?;
        let connection_cost_matrix =
            ConnectionCostMatrix::load(matrix_mtx).map_err(|_| AssetError::DictionaryDecode)?;
        let prefix_dictionary =
            PrefixDictionary::load(dict_da, dict_vals, dict_wordsidx, dict_words, true)
                .map_err(|_| AssetError::DictionaryDecode)?;
        let unknown_dictionary =
            UnknownDictionary::load(&unk_bin).map_err(|_| AssetError::DictionaryDecode)?;

        Ok(Dictionary {
            prefix_dictionary,
            connection_cost_matrix,
            character_definition,
            unknown_dictionary,
            metadata,
        })
    }
}

pub(crate) fn verify_voice(bytes: &[u8]) -> Result<(), AssetError> {
    if bytes.len() != LOCKED_VOICE_BYTES || !digest_matches(bytes, VOICE_SHA256) {
        return Err(AssetError::VoiceMismatch);
    }
    Ok(())
}

fn validate_dictionary_lengths(lengths: [usize; 8]) -> Result<usize, AssetError> {
    let mut total = 0usize;
    for length in lengths {
        if length == 0 || length > MAX_DICTIONARY_COMPONENT_BYTES {
            return Err(AssetError::DictionaryTooLarge);
        }
        total = total
            .checked_add(length)
            .ok_or(AssetError::DictionaryTooLarge)?;
        if total > MAX_DICTIONARY_TOTAL_BYTES {
            return Err(AssetError::DictionaryTooLarge);
        }
    }
    Ok(total)
}

fn digest_matches(bytes: &[u8], expected_hex: &str) -> bool {
    if expected_hex.len() != 64 {
        return false;
    }

    let digest = Sha256::digest(bytes);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for (index, byte) in digest.iter().enumerate() {
        let high = HEX[(byte >> 4) as usize];
        let low = HEX[(byte & 0x0f) as usize];
        let offset = index * 2;
        if expected_hex.as_bytes()[offset] != high || expected_hex.as_bytes()[offset + 1] != low {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::{
        digest_matches, validate_dictionary_lengths, AssetError, LOCKED_VOICE_BYTES,
        MAX_DICTIONARY_TOTAL_BYTES,
    };

    #[test]
    fn sha256_checker_matches_a_known_vector() {
        assert!(digest_matches(
            b"abc",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        ));
        assert!(!digest_matches(
            b"abd",
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        ));
    }

    #[test]
    fn non_pinned_voice_is_rejected_before_engine_parsing() {
        assert_eq!(super::verify_voice(&[]), Err(AssetError::VoiceMismatch));
    }

    #[test]
    fn dictionary_hash_mismatch_is_rejected_before_lindera_parsing() {
        let assets = super::DictionaryAssets {
            metadata_json: vec![0],
            char_def_bin: vec![0],
            matrix_mtx: vec![0],
            dict_da: vec![0],
            dict_vals: vec![0],
            dict_wordsidx: vec![0],
            dict_words: vec![0],
            unk_bin: vec![0],
        };
        assert!(matches!(
            assets.verify_and_decode(),
            Err(AssetError::DictionaryMismatch)
        ));
    }

    #[test]
    fn dictionary_size_gate_uses_checked_total_before_parsing() {
        assert!(validate_dictionary_lengths([1; 8]).is_ok());
        assert_eq!(
            validate_dictionary_lengths([MAX_DICTIONARY_TOTAL_BYTES, 1, 1, 1, 1, 1, 1, 1]),
            Err(AssetError::DictionaryTooLarge)
        );
        assert_eq!(
            validate_dictionary_lengths([usize::MAX, 1, 1, 1, 1, 1, 1, 1]),
            Err(AssetError::DictionaryTooLarge)
        );
        assert!(validate_dictionary_lengths([LOCKED_VOICE_BYTES, 1, 1, 1, 1, 1, 1, 1]).is_ok());
    }

    #[test]
    fn empty_or_single_oversized_dictionary_component_is_rejected() {
        assert_eq!(
            validate_dictionary_lengths([0, 1, 1, 1, 1, 1, 1, 1]),
            Err(AssetError::DictionaryTooLarge)
        );
        assert_eq!(
            validate_dictionary_lengths([MAX_DICTIONARY_TOTAL_BYTES + 1, 1, 1, 1, 1, 1, 1, 1]),
            Err(AssetError::DictionaryTooLarge)
        );
    }
}
