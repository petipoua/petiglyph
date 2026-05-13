fn main() {
    let c = '\u{100000}';
    println!("is_ws: {}", c.is_whitespace());
}
