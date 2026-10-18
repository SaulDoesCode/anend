use super::probe::probe;
use super::*;

/// the device, or None (and a note) where there is none: tests skip, they do not fail
pub(crate) fn device() -> Option<&'static Gpu> {
    match shared() {
        Ok(g) => Some(g),
        Err(e) => {
            eprintln!("skipping GPU test: {e}");
            None
        }
    }
}

#[test]
fn kernels_match_the_cpu() {
    let Some(g) = device() else { return };
    for prec in [Precision::Exact, Precision::Fast] {
        let rep = probe(g, prec);
        eprintln!("{}", rep.summary());
        assert!(rep.max_rel() < 1e-4, "{}", rep.summary());
    }
}
