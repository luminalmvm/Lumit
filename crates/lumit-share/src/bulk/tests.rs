use super::*;

#[test]
fn a_limit_makes_a_taker_wait() {
    let (limits, stop) = (Limits::default(), AtomicBool::new(false));
    let start = Instant::now();
    for _ in 0..20 {
        limits.up.take(PIECE, &stop);
    }
    assert!(start.elapsed() < Duration::from_millis(200));

    limits.set(20 * PIECE as u64, 0);
    std::thread::sleep(Duration::from_millis(50));
    let start = Instant::now();
    for _ in 0..40 {
        limits.up.take(PIECE, &stop);
    }
    assert!(start.elapsed() > Duration::from_millis(800));
    assert_eq!(limits.get(), (20 * PIECE as u64, 0));
}
