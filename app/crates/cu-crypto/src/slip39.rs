// SPDX-License-Identifier: GPL-3.0-or-later

use cu_msg::{Coded, Msg, display_as_msg};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::error::CryptoError;

const WORDLIST: &str = include_str!("slip39_wordlist.txt");

const RADIX_BITS: usize = 10;
const ID_BITS: usize = 15;
const EXTENDABLE_BITS: usize = 1;
const ITERATION_EXP_BITS: usize = 4;
const CHECKSUM_WORDS: usize = 3;
const METADATA_WORDS: usize = 4;
const MIN_SECRET_BYTES: usize = 16;
const MIN_MNEMONIC_WORDS: usize =
    METADATA_WORDS + CHECKSUM_WORDS + (MIN_SECRET_BYTES * 8).div_ceil(RADIX_BITS);
const DIGEST_LEN: usize = 4;
const DIGEST_INDEX: u8 = 254;
const SECRET_INDEX: u8 = 255;
const MAX_SHARES: u8 = 16;
const BASE_ITERATIONS: u32 = 10_000;
const ROUNDS: u8 = 4;

pub const DEFAULT_ITERATION_EXPONENT: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slip39Error {
    UnknownWord(usize),
    BadLength,
    BadChecksum,
    BadPadding,
    Mismatch,
    Duplicate,
    WrongCount { needed: usize, given: usize },
    BadDigest,
    BadParams(BadParam),
    RandomUnavailable,
}

impl Coded for Slip39Error {
    fn msg(&self) -> Msg {
        match self {
            Self::UnknownWord(position) => Msg::new("SLIP39_UNKNOWN_WORD").arg(position),
            Self::BadLength => Msg::new("SLIP39_BAD_LENGTH"),
            Self::BadChecksum => Msg::new("SLIP39_BAD_CHECKSUM"),
            Self::BadPadding => Msg::new("SLIP39_BAD_PADDING"),
            Self::Mismatch => Msg::new("SLIP39_MISMATCH"),
            Self::Duplicate => Msg::new("SLIP39_DUPLICATE"),
            Self::WrongCount { needed, given } => {
                Msg::new("SLIP39_WRONG_COUNT").arg(needed).arg(given)
            }
            Self::BadDigest => Msg::new("SLIP39_BAD_DIGEST"),
            Self::BadParams(BadParam::Threshold) => Msg::new("SLIP39_BAD_THRESHOLD"),
            Self::BadParams(BadParam::ThresholdOne) => Msg::new("SLIP39_THRESHOLD_ONE"),
            Self::BadParams(BadParam::SecretLength) => Msg::new("SLIP39_BAD_SECRET_LENGTH"),
            Self::BadParams(BadParam::Passphrase) => Msg::new("SLIP39_BAD_PASSPHRASE"),
            Self::RandomUnavailable => Msg::new("RANDOM_UNAVAILABLE"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadParam {
    Threshold,
    ThresholdOne,
    SecretLength,
    Passphrase,
}

display_as_msg!(Slip39Error);

impl std::error::Error for Slip39Error {}

impl From<CryptoError> for Slip39Error {
    fn from(_: CryptoError) -> Self {
        Self::RandomUnavailable
    }
}

fn word(index: u16) -> &'static str {
    WORDLIST.lines().nth(usize::from(index)).unwrap_or("")
}

fn word_index(candidate: &str) -> Option<u16> {
    WORDLIST
        .lines()
        .position(|w| w == candidate)
        .and_then(|i| u16::try_from(i).ok())
}

fn gf_mul(a: u8, b: u8) -> u8 {
    let mut a = a;
    let mut b = b;
    let mut product = 0u8;
    for _ in 0..8 {
        product ^= a & 0u8.wrapping_sub(b & 1);
        let carry = 0u8.wrapping_sub(a >> 7);
        a = (a << 1) ^ (0x1b & carry);
        b >>= 1;
    }
    product
}

fn gf_inv(a: u8) -> u8 {
    let mut power = a;
    for _ in 0..6 {
        power = gf_mul(power, power);
        power = gf_mul(power, a);
    }
    gf_mul(power, power)
}

struct Field;

const fn field() -> Field {
    Field
}

impl Field {
    #[allow(clippy::unused_self)]
    fn interpolate(&self, points: &[Point], x: u8) -> Zeroizing<Vec<u8>> {
        if let Some((_, value)) = points.iter().find(|(xi, _)| *xi == x) {
            return value.clone();
        }
        let len = points.first().map_or(0, |(_, y)| y.len());
        let mut result = Zeroizing::new(vec![0u8; len]);
        for (i, (xi, yi)) in points.iter().enumerate() {
            let mut numerator = 1u8;
            let mut denominator = 1u8;
            for (j, (xj, _)) in points.iter().enumerate() {
                if i != j {
                    numerator = gf_mul(numerator, x ^ xj);
                    denominator = gf_mul(denominator, xi ^ xj);
                }
            }
            let basis = gf_mul(numerator, gf_inv(denominator));
            for (out, &byte) in result.iter_mut().zip(yi.iter()) {
                *out ^= gf_mul(basis, byte);
            }
        }
        result
    }
}

fn rs1024_polymod(values: impl Iterator<Item = u32>) -> u32 {
    const GEN: [u32; 10] = [
        0x00e0_e040,
        0x01c1_c080,
        0x0383_8100,
        0x0707_0200,
        0x0e0e_0009,
        0x1c0c_2412,
        0x3808_6c24,
        0x3090_fc48,
        0x21b1_f890,
        0x03f3_f120,
    ];
    let mut chk: u32 = 1;
    for v in values {
        let b = chk >> 20;
        chk = ((chk & 0x000f_ffff) << 10) ^ v;
        for (i, g) in GEN.iter().enumerate() {
            if (b >> i) & 1 != 0 {
                chk ^= g;
            }
        }
    }
    chk
}

fn customization(extendable: bool) -> &'static str {
    if extendable {
        "shamir_extendable"
    } else {
        "shamir"
    }
}

fn checksum_values(extendable: bool, data: &[u16]) -> impl Iterator<Item = u32> + '_ {
    customization(extendable)
        .bytes()
        .map(u32::from)
        .chain(data.iter().map(|&w| u32::from(w)))
}

fn create_checksum(extendable: bool, data: &[u16]) -> [u16; CHECKSUM_WORDS] {
    let polymod = rs1024_polymod(checksum_values(extendable, data).chain([0, 0, 0])) ^ 1;
    let part = |shift: u32| u16::try_from((polymod >> shift) & 1023).unwrap_or(0);
    [part(20), part(10), part(0)]
}

fn verify_checksum(extendable: bool, data: &[u16]) -> bool {
    rs1024_polymod(checksum_values(extendable, data)) == 1
}

struct CipherParams<'a> {
    passphrase: &'a [u8],
    iteration_exponent: u8,
    identifier: u16,
    extendable: bool,
}

fn round_function(round: u8, params: &CipherParams<'_>, r: &[u8]) -> Zeroizing<Vec<u8>> {
    let mut password = Zeroizing::new(Vec::with_capacity(1 + params.passphrase.len()));
    password.push(round);
    password.extend_from_slice(params.passphrase);
    let mut salt = Zeroizing::new(Vec::new());
    if !params.extendable {
        salt.extend_from_slice(b"shamir");
        salt.extend_from_slice(&params.identifier.to_be_bytes());
    }
    salt.extend_from_slice(r);
    let iterations = (BASE_ITERATIONS << params.iteration_exponent) / u32::from(ROUNDS);
    let mut out = Zeroizing::new(vec![0u8; r.len()]);
    pbkdf2::pbkdf2_hmac::<Sha256>(&password, &salt, iterations, &mut out);
    out
}

fn feistel(
    input: &[u8],
    params: &CipherParams<'_>,
    rounds: impl Iterator<Item = u8>,
) -> Zeroizing<Vec<u8>> {
    let half = input.len() / 2;
    let (left, right) = input.split_at(half);
    let mut l = Zeroizing::new(left.to_vec());
    let mut r = Zeroizing::new(right.to_vec());
    for round in rounds {
        let f = round_function(round, params, &r);
        let next_r: Zeroizing<Vec<u8>> =
            Zeroizing::new(l.iter().zip(f.iter()).map(|(a, b)| a ^ b).collect());
        l = r;
        r = next_r;
    }
    let mut out = Zeroizing::new(Vec::with_capacity(input.len()));
    out.extend_from_slice(&r);
    out.extend_from_slice(&l);
    out
}

fn encrypt(secret: &[u8], params: &CipherParams<'_>) -> Zeroizing<Vec<u8>> {
    feistel(secret, params, 0..ROUNDS)
}

fn decrypt(encrypted: &[u8], params: &CipherParams<'_>) -> Zeroizing<Vec<u8>> {
    feistel(encrypted, params, (0..ROUNDS).rev())
}

type Point = (u8, Zeroizing<Vec<u8>>);

fn digest(random: &[u8], secret: &[u8]) -> Result<[u8; DIGEST_LEN], Slip39Error> {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(random)
        .map_err(|_| Slip39Error::BadParams(BadParam::SecretLength))?;
    mac.update(secret);
    let full = mac.finalize().into_bytes();
    let mut out = [0u8; DIGEST_LEN];
    for (o, b) in out.iter_mut().zip(full.iter()) {
        *o = *b;
    }
    Ok(out)
}

fn random_bytes(len: usize) -> Result<Zeroizing<Vec<u8>>, Slip39Error> {
    let mut out = Zeroizing::new(vec![0u8; len]);
    crate::random::fill_random(&mut out)?;
    Ok(out)
}

fn split_secret(
    gf: &Field,
    threshold: u8,
    count: u8,
    secret: &[u8],
) -> Result<Vec<Point>, Slip39Error> {
    if threshold == 1 {
        return Ok((0..count)
            .map(|i| (i, Zeroizing::new(secret.to_vec())))
            .collect());
    }
    let random_part = random_bytes(secret.len() - DIGEST_LEN)?;
    let mut digest_share = Zeroizing::new(digest(&random_part, secret)?.to_vec());
    digest_share.extend_from_slice(&random_part);

    let mut base: Vec<Point> = Vec::new();
    for i in 0..threshold - 2 {
        base.push((i, random_bytes(secret.len())?));
    }
    base.push((DIGEST_INDEX, digest_share));
    base.push((SECRET_INDEX, Zeroizing::new(secret.to_vec())));

    let mut shares: Vec<Point> = base
        .iter()
        .filter(|(x, _)| *x < threshold - 2)
        .cloned()
        .collect();
    for i in threshold - 2..count {
        shares.push((i, gf.interpolate(&base, i)));
    }
    Ok(shares)
}

fn recover_secret(
    gf: &Field,
    threshold: u8,
    shares: &[Point],
) -> Result<Zeroizing<Vec<u8>>, Slip39Error> {
    if threshold == 1 {
        return shares
            .first()
            .map(|(_, value)| value.clone())
            .ok_or(Slip39Error::BadLength);
    }
    let secret = gf.interpolate(shares, SECRET_INDEX);
    let digest_share = gf.interpolate(shares, DIGEST_INDEX);
    let (expected, random_part) = digest_share
        .split_at_checked(DIGEST_LEN)
        .ok_or(Slip39Error::BadLength)?;
    let actual = digest(random_part, &secret)?;
    if subtle::ConstantTimeEq::ct_eq(expected, actual.as_slice()).into() {
        Ok(secret)
    } else {
        Err(Slip39Error::BadDigest)
    }
}

struct Share {
    identifier: u16,
    extendable: bool,
    iteration_exponent: u8,
    group_index: u8,
    group_threshold: u8,
    group_count: u8,
    member_index: u8,
    member_threshold: u8,
    value: Zeroizing<Vec<u8>>,
}

struct BitWriter {
    words: Vec<u16>,
    acc: u32,
    acc_bits: usize,
}

impl BitWriter {
    fn push(&mut self, value: u32, bits: usize) {
        for shift in (0..bits).rev() {
            self.acc = (self.acc << 1) | ((value >> shift) & 1);
            self.acc_bits += 1;
            if self.acc_bits == RADIX_BITS {
                self.words.push(u16::try_from(self.acc).unwrap_or(0));
                self.acc = 0;
                self.acc_bits = 0;
            }
        }
    }
}

fn encode(share: &Share) -> String {
    let mut writer = BitWriter {
        words: Vec::new(),
        acc: 0,
        acc_bits: 0,
    };
    writer.push(u32::from(share.identifier), ID_BITS);
    writer.push(u32::from(share.extendable), EXTENDABLE_BITS);
    writer.push(u32::from(share.iteration_exponent), ITERATION_EXP_BITS);
    writer.push(u32::from(share.group_index), 4);
    writer.push(u32::from(share.group_threshold - 1), 4);
    writer.push(u32::from(share.group_count - 1), 4);
    writer.push(u32::from(share.member_index), 4);
    writer.push(u32::from(share.member_threshold - 1), 4);
    let value_bits = share.value.len() * 8;
    writer.push(0, (RADIX_BITS - value_bits % RADIX_BITS) % RADIX_BITS);
    for &byte in share.value.iter() {
        writer.push(u32::from(byte), 8);
    }
    let checksum = create_checksum(share.extendable, &writer.words);
    writer
        .words
        .iter()
        .chain(checksum.iter())
        .map(|&w| word(w))
        .collect::<Vec<_>>()
        .join(" ")
}

fn read_bits(words: &[u16], cursor: &mut usize, bits: usize) -> u32 {
    let mut out = 0u32;
    for _ in 0..bits {
        let word = words.get(*cursor / RADIX_BITS).copied().unwrap_or(0);
        let bit = (word >> (RADIX_BITS - 1 - *cursor % RADIX_BITS)) & 1;
        out = (out << 1) | u32::from(bit);
        *cursor += 1;
    }
    out
}

fn small(value: u32) -> u8 {
    u8::try_from(value).unwrap_or(u8::MAX)
}

fn decode(mnemonic: &str) -> Result<Share, Slip39Error> {
    let mut words = Vec::new();
    for (position, candidate) in mnemonic.split_whitespace().enumerate() {
        let lowered = candidate.to_ascii_lowercase();
        words.push(word_index(&lowered).ok_or(Slip39Error::UnknownWord(position + 1))?);
    }
    if words.len() < MIN_MNEMONIC_WORDS {
        return Err(Slip39Error::BadLength);
    }
    let padding_bits = (RADIX_BITS * (words.len() - METADATA_WORDS - CHECKSUM_WORDS)) % 16;
    if padding_bits > 8 {
        return Err(Slip39Error::BadLength);
    }

    let mut cursor = 0;
    let identifier = u16::try_from(read_bits(&words, &mut cursor, ID_BITS)).unwrap_or(0);
    let extendable = read_bits(&words, &mut cursor, EXTENDABLE_BITS) == 1;
    let iteration_exponent = small(read_bits(&words, &mut cursor, ITERATION_EXP_BITS));
    if !verify_checksum(extendable, &words) {
        return Err(Slip39Error::BadChecksum);
    }
    let group_index = small(read_bits(&words, &mut cursor, 4));
    let group_threshold = small(read_bits(&words, &mut cursor, 4)) + 1;
    let group_count = small(read_bits(&words, &mut cursor, 4)) + 1;
    let member_index = small(read_bits(&words, &mut cursor, 4));
    let member_threshold = small(read_bits(&words, &mut cursor, 4)) + 1;
    if group_count < group_threshold {
        return Err(Slip39Error::Mismatch);
    }
    if read_bits(&words, &mut cursor, padding_bits) != 0 {
        return Err(Slip39Error::BadPadding);
    }
    let value_words = words
        .get(..words.len() - CHECKSUM_WORDS)
        .ok_or(Slip39Error::BadLength)?;
    let value_bytes = (value_words.len() * RADIX_BITS - cursor) / 8;
    let mut value = Zeroizing::new(Vec::with_capacity(value_bytes));
    for _ in 0..value_bytes {
        value.push(small(read_bits(value_words, &mut cursor, 8)));
    }
    Ok(Share {
        identifier,
        extendable,
        iteration_exponent,
        group_index,
        group_threshold,
        group_count,
        member_index,
        member_threshold,
        value,
    })
}

pub fn split(
    secret: &[u8],
    threshold: u8,
    count: u8,
    passphrase: &str,
) -> Result<Vec<Zeroizing<String>>, Slip39Error> {
    if threshold == 0 || threshold > count || count > MAX_SHARES {
        return Err(Slip39Error::BadParams(BadParam::Threshold));
    }
    if threshold == 1 && count > 1 {
        return Err(Slip39Error::BadParams(BadParam::ThresholdOne));
    }
    if secret.len() < MIN_SECRET_BYTES || !secret.len().is_multiple_of(2) {
        return Err(Slip39Error::BadParams(BadParam::SecretLength));
    }
    if !passphrase.bytes().all(|b| (32..=126).contains(&b)) {
        return Err(Slip39Error::BadParams(BadParam::Passphrase));
    }
    let mut id_bytes = [0u8; 2];
    crate::random::fill_random(&mut id_bytes)?;
    let identifier = u16::from_be_bytes(id_bytes) & 0x7fff;
    let params = CipherParams {
        passphrase: passphrase.as_bytes(),
        iteration_exponent: DEFAULT_ITERATION_EXPONENT,
        identifier,
        extendable: true,
    };
    let encrypted = encrypt(secret, &params);
    let gf = field();
    Ok(split_secret(&gf, threshold, count, &encrypted)?
        .into_iter()
        .map(|(member_index, value)| {
            Zeroizing::new(encode(&Share {
                identifier,
                extendable: true,
                iteration_exponent: DEFAULT_ITERATION_EXPONENT,
                group_index: 0,
                group_threshold: 1,
                group_count: 1,
                member_index,
                member_threshold: threshold,
                value,
            }))
        })
        .collect())
}

pub fn combine(mnemonics: &[&str], passphrase: &str) -> Result<Zeroizing<Vec<u8>>, Slip39Error> {
    let shares: Vec<Share> = mnemonics
        .iter()
        .map(|m| decode(m))
        .collect::<Result<_, _>>()?;
    let first = shares.first().ok_or(Slip39Error::WrongCount {
        needed: 1,
        given: 0,
    })?;
    if shares.iter().any(|s| {
        s.identifier != first.identifier
            || s.extendable != first.extendable
            || s.iteration_exponent != first.iteration_exponent
            || s.group_threshold != first.group_threshold
            || s.group_count != first.group_count
            || s.value.len() != first.value.len()
    }) {
        return Err(Slip39Error::Mismatch);
    }

    let mut group_indices: Vec<u8> = shares.iter().map(|s| s.group_index).collect();
    group_indices.sort_unstable();
    group_indices.dedup();
    if group_indices.len() != usize::from(first.group_threshold) {
        return Err(Slip39Error::WrongCount {
            needed: usize::from(first.group_threshold),
            given: group_indices.len(),
        });
    }

    let gf = field();
    let mut group_shares = Vec::new();
    for group_index in group_indices {
        let members: Vec<&Share> = shares
            .iter()
            .filter(|s| s.group_index == group_index)
            .collect();
        let threshold = members.first().map_or(1, |s| s.member_threshold);
        if members.iter().any(|s| s.member_threshold != threshold) {
            return Err(Slip39Error::Mismatch);
        }
        let mut member_indices: Vec<u8> = members.iter().map(|s| s.member_index).collect();
        member_indices.sort_unstable();
        member_indices.dedup();
        if member_indices.len() != members.len() {
            return Err(Slip39Error::Duplicate);
        }
        if members.len() != usize::from(threshold) {
            return Err(Slip39Error::WrongCount {
                needed: usize::from(threshold),
                given: members.len(),
            });
        }
        let points: Vec<Point> = members
            .iter()
            .map(|s| (s.member_index, s.value.clone()))
            .collect();
        group_shares.push((group_index, recover_secret(&gf, threshold, &points)?));
    }
    let encrypted = recover_secret(&gf, first.group_threshold, &group_shares)?;
    Ok(decrypt(
        &encrypted,
        &CipherParams {
            passphrase: passphrase.as_bytes(),
            iteration_exponent: first.iteration_exponent,
            identifier: first.identifier,
            extendable: first.extendable,
        },
    ))
}
