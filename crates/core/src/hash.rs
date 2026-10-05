//! xxh64, usable in constant evaluation so name-derived keys are computed at compile time.

const P1: u64 = 0x9E37_79B1_85EB_CA87;
const P2: u64 = 0xC2B2_AE3D_27D4_EB4F;
const P3: u64 = 0x1656_67B1_9E37_79F9;
const P4: u64 = 0x85EB_CA77_C2B2_AE63;
const P5: u64 = 0x27D4_EB2F_1656_67C5;

const fn read_u64(data: &[u8], i: usize) -> u64 {
    u64::from_le_bytes([
        data[i],
        data[i + 1],
        data[i + 2],
        data[i + 3],
        data[i + 4],
        data[i + 5],
        data[i + 6],
        data[i + 7],
    ])
}

const fn read_u32(data: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]])
}

const fn round(acc: u64, input: u64) -> u64 {
    acc.wrapping_add(input.wrapping_mul(P2)).rotate_left(31).wrapping_mul(P1)
}

const fn merge_round(hash: u64, value: u64) -> u64 {
    (hash ^ round(0, value)).wrapping_mul(P1).wrapping_add(P4)
}

/// The 64-bit xxHash of `data` with `seed`, identical to the reference implementation.
#[must_use]
pub const fn xxh64(data: &[u8], seed: u64) -> u64 {
    let len = data.len();
    let mut i = 0;
    let mut hash;

    if len >= 32 {
        let mut v1 = seed.wrapping_add(P1).wrapping_add(P2);
        let mut v2 = seed.wrapping_add(P2);
        let mut v3 = seed;
        let mut v4 = seed.wrapping_sub(P1);
        while i + 32 <= len {
            v1 = round(v1, read_u64(data, i));
            v2 = round(v2, read_u64(data, i + 8));
            v3 = round(v3, read_u64(data, i + 16));
            v4 = round(v4, read_u64(data, i + 24));
            i += 32;
        }
        hash = v1
            .rotate_left(1)
            .wrapping_add(v2.rotate_left(7))
            .wrapping_add(v3.rotate_left(12))
            .wrapping_add(v4.rotate_left(18));
        hash = merge_round(hash, v1);
        hash = merge_round(hash, v2);
        hash = merge_round(hash, v3);
        hash = merge_round(hash, v4);
    } else {
        hash = seed.wrapping_add(P5);
    }

    hash = hash.wrapping_add(len as u64);

    while i + 8 <= len {
        hash ^= round(0, read_u64(data, i));
        hash = hash.rotate_left(27).wrapping_mul(P1).wrapping_add(P4);
        i += 8;
    }
    if i + 4 <= len {
        hash ^= (read_u32(data, i) as u64).wrapping_mul(P1);
        hash = hash.rotate_left(23).wrapping_mul(P2).wrapping_add(P3);
        i += 4;
    }
    while i < len {
        hash ^= (data[i] as u64).wrapping_mul(P5);
        hash = hash.rotate_left(11).wrapping_mul(P1);
        i += 1;
    }

    hash ^= hash >> 33;
    hash = hash.wrapping_mul(P2);
    hash ^= hash >> 29;
    hash = hash.wrapping_mul(P3);
    hash ^= hash >> 32;
    hash
}

#[cfg(test)]
mod tests {
    use super::xxh64;

    #[test]
    fn known_vectors() {
        assert_eq!(xxh64(b"", 0), 0xEF46_DB37_51D8_E999);
        assert_eq!(xxh64(b"a", 0), 0xD24E_C4F1_A98C_6E5B);
        assert_eq!(xxh64(b"abc", 0), 0x44BC_2CF5_AD77_0999);
        assert_eq!(xxh64(b"Nobody inspects the spammish repetition", 0), 0xFBCE_A83C_8A37_8BF1);
    }

    #[test]
    fn matches_reference_at_every_length() {
        let data: Vec<u8> =
            (0..300u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
        for len in 0..data.len() {
            for seed in [0, 1, 0xDEAD_BEEF_CAFE_F00D] {
                assert_eq!(
                    xxh64(&data[..len], seed),
                    xxhash_rust::xxh64::xxh64(&data[..len], seed),
                    "length {len}, seed {seed}"
                );
            }
        }
    }

    #[test]
    fn usable_in_const() {
        const KEY: u64 = xxh64(b"game_sim::apply_gravity", 0);
        assert_eq!(KEY, xxh64(b"game_sim::apply_gravity", 0));
    }
}
