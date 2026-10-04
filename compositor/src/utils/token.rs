const TOKEN_BYTES: usize = 32;
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";

/// 32 bytes from the kernel's CSPRNG as 64 lowercase hex digits.
pub fn random_hex_token() -> Result<String, getrandom::Error> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes)?;
    Ok(bytes
        .iter()
        .flat_map(|byte| [byte >> 4, byte & 0xf])
        .map(|nibble| char::from(HEX_DIGITS[usize::from(nibble)]))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_64_lowercase_hex_digits() -> Result<(), getrandom::Error> {
        let token = random_hex_token()?;
        assert_eq!(token.len(), TOKEN_BYTES * 2);
        assert!(token.bytes().all(|byte| HEX_DIGITS.contains(&byte)));
        Ok(())
    }

    #[test]
    fn tokens_do_not_repeat() -> Result<(), getrandom::Error> {
        assert_ne!(random_hex_token()?, random_hex_token()?);
        Ok(())
    }
}
