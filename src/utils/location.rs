use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub struct Location {
    pub line: usize,
    pub col: usize,
    pub file: Rc<str>,
}
impl Location {
    pub fn new_with_file(line: usize, col: usize, file: Rc<str>) -> Self {
        Self {
            line: Self::error_correct_line(file.clone(), line),
            col,
            file,
        }
    }

    fn error_correct_line(file: Rc<str>, line: usize) -> usize {
        let contents = match std::fs::read_to_string(&*file) {
            Ok(contents) => contents,
            Err(_) => return line + 1,
        };

        contents
            .lines()
            .enumerate()
            .skip(line)
            .find(|(_, line)| !line.trim().is_empty())
            .map(|(line_number, _)| line_number + 1)
            .unwrap_or(line + 1)
    }
}
impl std::fmt::Display for Location {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "li = {}, co = {}", self.line, self.col)
    }
}
