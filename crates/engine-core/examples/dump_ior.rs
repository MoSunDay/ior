use engine_core::dict::Dictionary;

fn main() {
    let path = std::env::args().nth(1).expect("usage: dump_ior <file.ior>");
    let bytes = std::fs::read(&path).expect("read .ior");
    let dict = Dictionary::from_ior_bytes(&bytes).expect("parse .ior");
    let mut rows: Vec<(String, String, u32)> = dict
        .iter()
        .flat_map(|(key, vals)| {
            vals.iter()
                .map(move |(h, f)| (key.to_string(), h.clone(), *f))
        })
        .collect();
    rows.sort();
    for (key, hanzi, freq) in rows {
        println!("{key}\t{hanzi}\t{freq}");
    }
}
