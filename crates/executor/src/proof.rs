//! Either side shows the other that it has the key, which itself is never
//! sent: whoever took the place of either end learns nothing to use.

use hmac::{Hmac, KeyInit as _, Mac as _};
use sha2::Sha256;

/// Which side a proof is of, so that neither can give back the other's.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Side {
    Asks,
    Answers,
}

/// Something never used before.
pub(crate) fn nonce() -> String {
    format!("{:032x}", rand::random::<u128>())
}

fn mac(key: &str, side: Side, asks: &str, answers: &str) -> Hmac<Sha256> {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(key.as_bytes()).expect("a key of any length is one");
    let side = match side {
        Side::Asks => "asks",
        Side::Answers => "answers",
    };
    for part in [side, asks, answers] {
        mac.update(part.as_bytes());
        mac.update(&[0]);
    }
    mac
}

/// The proof of `side`, for the nonces both made.
pub(crate) fn of(key: &str, side: Side, asks: &str, answers: &str) -> String {
    let proof = mac(key, side, asks, answers).finalize().into_bytes();
    proof.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn holds(key: &str, side: Side, asks: &str, answers: &str, proof: &str) -> bool {
    let bytes: Option<Vec<u8>> = proof
        .as_bytes()
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect();
    bytes.is_some_and(|bytes| mac(key, side, asks, answers).verify_slice(&bytes).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_proof_holds_for_its_key_side_and_nonces_alone() {
        let proof = of("k", Side::Asks, "a", "b");
        assert!(holds("k", Side::Asks, "a", "b", &proof));
        for (key, side, asks, answers) in [
            ("x", Side::Asks, "a", "b"),
            ("k", Side::Answers, "a", "b"),
            ("k", Side::Asks, "c", "b"),
            ("k", Side::Asks, "a", "c"),
        ] {
            assert!(!holds(key, side, asks, answers, &proof));
        }
        for wrong in ["", "zz", "é", &proof[2..]] {
            assert!(!holds("k", Side::Asks, "a", "b", wrong), "{wrong}");
        }
    }
}
