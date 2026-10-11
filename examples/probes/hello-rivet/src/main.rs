//! Prints a line and sleeps, for ever. The point is that it RUNS.

fn main() {
    let mut n: u64 = 0;
    loop {
        println!("hello-rivet: tick {n}");
        n += 1;
        std::thread::sleep(std::time::Duration::from_secs(30));
    }
}
