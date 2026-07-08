use crate::{lexing::lexing::Token, parse::parsing::Identifier};

pub fn to_ident(token: Option<Token>) -> Option<Identifier> {
    if let Some(tk) = token {
        return Some(Identifier {
            value: tk.value,
            location: tk.location,
        });
    }
    None
}
