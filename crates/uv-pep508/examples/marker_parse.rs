//! Time one marker parse in a fresh process, before the global interner is populated.

use std::error::Error;
use std::hint::black_box;
use std::io::{self, Read};
use std::str::FromStr;
use std::time::Instant;

use uv_pep508::MarkerTree;

#[expect(
    clippy::print_stdout,
    reason = "the benchmark driver reads the parse duration"
)]
fn main() -> Result<(), Box<dyn Error>> {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;
    let start = Instant::now();
    let marker = MarkerTree::from_str(black_box(input.as_str()))?;
    let elapsed = start.elapsed();
    black_box(marker);
    println!("{}", elapsed.as_nanos());
    Ok(())
}
