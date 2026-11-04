#!/usr/bin/env python3
"""
Utilitário para empacotar binários embutidos no ChronoKeep.
Aplica compressão zlib e máscara XOR de byte a byte para evitar falsos positivos
de antivírus (como Windows Defender Dropper/Trojan) causados pela presença de
cabeçalhos PE brutos (MZ) e símbolos de hooking de memória expostos no executável final.
"""

import sys
import zlib

MASK_KEY = b"ChronoKeepProtectedAssetV1"

def pack_file(src_path: str, dst_path: str):
    with open(src_path, "rb") as f:
        data = f.read()

    # Comprime com o nível máximo de compressão
    compressed = zlib.compress(data, 9)

    # Aplica máscara XOR para eliminar assinaturas e cabeçalhos residuais
    key_len = len(MASK_KEY)
    masked = bytearray(len(compressed))
    for i in range(len(compressed)):
        masked[i] = compressed[i] ^ MASK_KEY[i % key_len]

    with open(dst_path, "wb") as f:
        f.write(masked)

    print(f"[*] Empacotado: {src_path} ({len(data)} bytes) -> {dst_path} ({len(masked)} bytes)")

if __name__ == "__main__":
    if len(sys.argv) != 3:
        print("Uso: python3 scripts/pack_embedded.py <arquivo_origem> <arquivo_destino>")
        sys.exit(1)
    pack_file(sys.argv[1], sys.argv[2])
