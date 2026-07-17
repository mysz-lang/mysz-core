#[derive(Clone, Debug)]
pub struct Location {
    pub line: usize,
    pub col: usize,
}
impl Location {
    pub fn new(line: usize, col: usize) -> Self {
        Self { line, col }
    }
}
impl std::fmt::Display for Location {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "li = {}, co = {}", self.line + 1, self.col)
    }
}
