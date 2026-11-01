use engine_core::dict::Dictionary;
use std::time::Instant;

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: load_only <file.ior>");
    let bytes = std::fs::read(&path).expect("read .ior");
    let start = Instant::now();
    let dict = Dictionary::from_ior_bytes(&bytes).expect("parse .ior");
    println!("keys={} load={:?}", dict.len(), start.elapsed());
}
