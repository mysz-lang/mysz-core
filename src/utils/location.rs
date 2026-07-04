#[derive(Clone)]
pub struct Location {
    pub line: usize,
    pub col: usize,
}
impl Location {
    pub fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
}