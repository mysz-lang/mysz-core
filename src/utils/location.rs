use std::rc::Rc;

#[derive(Clone, Debug)]
pub struct Location {
    pub line: usize,
    pub col: usize,
    pub file: Rc<str>,
}
impl Location {
    pub fn new_with_file(line: usize, col: usize, file: Rc<str>) -> Self {
        Self { line, col, file }
    }
}
impl std::fmt::Display for Location {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "li = {}, co = {}", self.line, self.col)
    }
}
