//! Reads a shared framebuffer file (produced by the compositor) and writes a PNG snapshot.

use std::fs::File;
use std::io::BufWriter;

fn main() {
    let mut args = std::env::args().skip(1);
    let (Some(input), Some(output)) = (args.next(), args.next()) else {
        eprintln!("usage: fb-dump <shared-framebuffer-file> <out.png>");
        std::process::exit(2);
    };

    let file = File::open(&input).unwrap_or_else(|e| {
        eprintln!("open {input}: {e}");
        std::process::exit(1);
    });
    // SAFETY: read-only mapping; the compositor may write concurrently, which is tolerated.
    let map = unsafe {
        memmap2::Mmap::map(&file).unwrap_or_else(|e| {
            eprintln!("mmap {input}: {e}");
            std::process::exit(1);
        })
    };

    let (w, h, rgba) = fb_dump::snapshot(&map).unwrap_or_else(|| {
        eprintln!("{input}: not a valid shared framebuffer (bad/absent header)");
        std::process::exit(1);
    });

    let out = File::create(&output).unwrap_or_else(|e| {
        eprintln!("create {output}: {e}");
        std::process::exit(1);
    });
    let mut encoder = png::Encoder::new(BufWriter::new(out), w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .and_then(|mut w| w.write_image_data(&rgba))
        .unwrap_or_else(|e| {
            eprintln!("write png: {e}");
            std::process::exit(1);
        });

    println!("wrote {output} ({w}x{h})");
}
