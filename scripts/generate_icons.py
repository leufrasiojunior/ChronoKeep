#!/usr/bin/env python3
"""
scripts/generate_icons.py
Gera icones multi-resolucao (.ico) e .png com mascara squircle (estilo Fluent Design Windows 11)
a partir de uma imagem fonte, distribuindo para cr-gui e icones_gerados.
"""

import ctypes
import math
import os
import struct
import sys

def main():
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    src_jpg = os.path.join(root, "icones_gerados/chronokeep_source.jpg")
    if not os.path.isfile(src_jpg):
        print(f"Erro: arquivo fonte nao encontrado: {src_jpg}")
        sys.exit(1)

    gdk = ctypes.CDLL("libgdk_pixbuf-2.0.so.0")
    gdk.gdk_pixbuf_new_from_file.restype = ctypes.c_void_p
    gdk.gdk_pixbuf_get_pixels.restype = ctypes.POINTER(ctypes.c_uint8)
    gdk.gdk_pixbuf_new.restype = ctypes.c_void_p
    gdk.gdk_pixbuf_scale_simple.restype = ctypes.c_void_p
    gdk.gdk_pixbuf_save.restype = ctypes.c_int

    pb_src = gdk.gdk_pixbuf_new_from_file(src_jpg.encode("utf-8"), None)
    src_w = gdk.gdk_pixbuf_get_width(pb_src)
    src_h = gdk.gdk_pixbuf_get_height(pb_src)
    src_stride = gdk.gdk_pixbuf_get_rowstride(pb_src)
    src_pix = gdk.gdk_pixbuf_get_pixels(pb_src)

    def get_src_pixel(x, y):
        x = max(0, min(src_w - 1, x))
        y = max(0, min(src_h - 1, y))
        idx = y * src_stride + x * 3
        return (src_pix[idx], src_pix[idx+1], src_pix[idx+2])

    crop_cx, crop_cy = 1024, 1030
    crop_size = 1490
    crop_x0 = crop_cx - crop_size // 2
    crop_y0 = crop_cy - crop_size // 2

    master_size = 512
    pb_master = gdk.gdk_pixbuf_new(0, True, 8, master_size, master_size)
    master_stride = gdk.gdk_pixbuf_get_rowstride(pb_master)
    master_pix = gdk.gdk_pixbuf_get_pixels(pb_master)

    margin = 6.0
    inner_r = 110.0

    for y in range(master_size):
        for x in range(master_size):
            sx = crop_x0 + (x / float(master_size)) * crop_size
            sy = crop_y0 + (y / float(master_size)) * crop_size
            
            px = abs(x - master_size / 2.0) - (master_size / 2.0 - margin - inner_r)
            py = abs(y - master_size / 2.0) - (master_size / 2.0 - margin - inner_r)
            
            if px > 0 and py > 0:
                dist = math.sqrt(px*px + py*py) - inner_r
            else:
                dist = max(px - inner_r, py - inner_r)
                
            if dist <= -0.5:
                alpha = 1.0
            elif dist >= 0.5:
                alpha = 0.0
            else:
                alpha = 0.5 - dist
                
            x0, y0 = int(sx), int(sy)
            x1, y1 = x0 + 1, y0 + 1
            fx, fy = sx - x0, sy - y0
            
            c00 = get_src_pixel(x0, y0)
            c10 = get_src_pixel(x1, y0)
            c01 = get_src_pixel(x0, y1)
            c11 = get_src_pixel(x1, y1)
            
            r = (1-fx)*(1-fy)*c00[0] + fx*(1-fy)*c10[0] + (1-fx)*fy*c01[0] + fx*fy*c11[0]
            g = (1-fx)*(1-fy)*c00[1] + fx*(1-fy)*c10[1] + (1-fx)*fy*c01[1] + fx*fy*c11[1]
            b = (1-fx)*(1-fy)*c00[2] + fx*(1-fy)*c10[2] + (1-fx)*fy*c01[2] + fx*fy*c11[2]
            
            out_idx = y * master_stride + x * 4
            master_pix[out_idx] = int(round(r))
            master_pix[out_idx+1] = int(round(g))
            master_pix[out_idx+2] = int(round(b))
            master_pix[out_idx+3] = int(round(alpha * 255.0))

    # Master PNGs
    os.makedirs(os.path.join(root, "icones_gerados"), exist_ok=True)
    os.makedirs(os.path.join(root, "rust-workspace/crates/cr-gui/icons"), exist_ok=True)
    os.makedirs(os.path.join(root, "rust-workspace/crates/cr-gui/assets"), exist_ok=True)
    os.makedirs(os.path.join(root, "rust-workspace/crates/cr-gui/ui"), exist_ok=True)

    gdk.gdk_pixbuf_save(pb_master, os.path.join(root, "icones_gerados/app-logo-512.png").encode(), b"png", None, None)

    pb_256 = gdk.gdk_pixbuf_scale_simple(pb_master, 256, 256, 3)
    gdk.gdk_pixbuf_save(pb_256, os.path.join(root, "icones_gerados/app-logo-256.png").encode(), b"png", None, None)
    gdk.gdk_pixbuf_save(pb_256, os.path.join(root, "icones_gerados/app-logo.png").encode(), b"png", None, None)
    gdk.gdk_pixbuf_save(pb_256, os.path.join(root, "rust-workspace/crates/cr-gui/ui/app-logo.png").encode(), b"png", None, None)
    gdk.gdk_pixbuf_save(pb_256, os.path.join(root, "rust-workspace/crates/cr-gui/icons/icon.png").encode(), b"png", None, None)

    pb_128 = gdk.gdk_pixbuf_scale_simple(pb_master, 128, 128, 3)
    gdk.gdk_pixbuf_save(pb_128, os.path.join(root, "rust-workspace/crates/cr-gui/icons/128x128.png").encode(), b"png", None, None)
    gdk.gdk_pixbuf_save(pb_256, os.path.join(root, "rust-workspace/crates/cr-gui/icons/128x128@2x.png").encode(), b"png", None, None)

    pb_32 = gdk.gdk_pixbuf_scale_simple(pb_master, 32, 32, 3)
    gdk.gdk_pixbuf_save(pb_32, os.path.join(root, "rust-workspace/crates/cr-gui/icons/32x32.png").encode(), b"png", None, None)

    sizes = [256, 128, 64, 48, 32, 24, 16]
    ico_entries = []
    for s in sizes:
        pb_s = gdk.gdk_pixbuf_scale_simple(pb_master, s, s, 3)
        tmp_path = f"/tmp/ico_{s}.png"
        gdk.gdk_pixbuf_save(pb_s, tmp_path.encode(), b"png", None, None)
        with open(tmp_path, "rb") as f:
            ico_entries.append((s, f.read()))

    def write_ico(entries, out_path):
        count = len(entries)
        header = struct.pack("<HHH", 0, 1, count)
        dir_entries = []
        offset = 6 + count * 16
        for s, data in entries:
            wb = 0 if s >= 256 else s
            hb = 0 if s >= 256 else s
            dir_entries.append(struct.pack("<BBBBHHII", wb, hb, 0, 0, 1, 32, len(data), offset))
            offset += len(data)
        with open(out_path, "wb") as f:
            f.write(header)
            for e in dir_entries:
                f.write(e)
            for s, data in entries:
                f.write(data)
        print(f"Gravado {out_path} ({os.path.getsize(out_path)} bytes)")

    write_ico(ico_entries, os.path.join(root, "icones_gerados/chronokeep_app.ico"))
    write_ico(ico_entries, os.path.join(root, "icones_gerados/chronokeep_app_escuro.ico"))
    write_ico(ico_entries, os.path.join(root, "rust-workspace/crates/cr-gui/assets/app.ico"))
    write_ico(ico_entries, os.path.join(root, "rust-workspace/crates/cr-gui/icons/icon.ico"))
    write_ico(ico_entries, os.path.join(root, "rust-workspace/crates/cr-gui/ui/favicon.ico"))
    print("Processamento de icones concluido com sucesso!")

if __name__ == "__main__":
    main()

