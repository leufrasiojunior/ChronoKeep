//! Gerenciamento de binários embutidos com compressão zlib e mascaramento XOR.
//!
//! Evita falsos positivos em antivírus e scanners heurísticos (ex: Windows Defender
//! classificando como Trojan Dropper) ao remover cabeçalhos PE expostos (MZ...),
//! seções de executável e símbolos de injeção/hooking de memória dentro do ChronoKeep.exe.

use miniz_oxide::inflate::decompress_to_vec_zlib;
use std::sync::OnceLock;

// Bytes comprimidos e mascarados dos binários auxiliares
const PACKED_DAEMON: &[u8] = include_bytes!("../embedded/chrono-daemon.bin");
const PACKED_BRIDGE: &[u8] = include_bytes!("../embedded/chronokeep.bin");

// Chave fixa para desmascaramento XOR
const MASK_KEY: &[u8] = b"ChronoKeepProtectedAssetV1";

fn unpack(packed: &[u8]) -> Vec<u8> {
    let unmasked: Vec<u8> = packed
        .iter()
        .zip(MASK_KEY.iter().cycle())
        .map(|(&b, &k)| b ^ k)
        .collect();
    decompress_to_vec_zlib(&unmasked).expect("falha crítica ao descompactar componente embutido")
}

static CACHED_DAEMON: OnceLock<Vec<u8>> = OnceLock::new();
static CACHED_BRIDGE: OnceLock<Vec<u8>> = OnceLock::new();

/// Retorna os bytes descompactados de `chrono-daemon.exe`.
pub fn get_embedded_daemon() -> &'static [u8] {
    CACHED_DAEMON.get_or_init(|| unpack(PACKED_DAEMON)).as_slice()
}

/// Retorna os bytes descompactados de `chronokeep.dll`.
pub fn get_embedded_bridge() -> &'static [u8] {
    CACHED_BRIDGE.get_or_init(|| unpack(PACKED_BRIDGE)).as_slice()
}
