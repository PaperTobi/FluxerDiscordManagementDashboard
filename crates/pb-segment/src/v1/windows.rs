/// A part of a long clip that is scored on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub start: usize,
    pub len: usize,
}

/// Splits `samples` into windows of at most `max` with starts `hop` apart, the last one ending at the clip's end
/// (the classifier reads at most 30 s; longer speech is scored in 30 s windows with a 25 s hop and the results
/// combined). A clip that fits is one window.
pub fn windows(samples: usize, max: usize, hop: usize) -> Vec<Window> {
    assert!(max > 0 && hop > 0 && hop <= max, "hop must be within the window");
    let mut out = Vec::new();
    let mut start = 0;
    loop {
        let len = max.min(samples - start);
        out.push(Window { start, len });
        if start + max >= samples {
            break;
        }
        start += hop;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_like_the_reference() {
        assert_eq!(windows(100, 480, 400), vec![Window { start: 0, len: 100 }]);
        assert_eq!(windows(480, 480, 400), vec![Window { start: 0, len: 480 }]);
        assert_eq!(
            windows(528, 480, 400),
            vec![Window { start: 0, len: 480 }, Window { start: 400, len: 128 }]
        );
    }
}
