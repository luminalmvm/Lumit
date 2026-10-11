//! Turning tokens into a tree the interpreter walks.
//!
//! The grammar is the part of JavaScript that After Effects expressions are
//! written in: statements, functions, loops, arrays and objects. Classes,
//! generators, regular expressions and modules are left out, and an expression
//! that uses one is refused at this stage with the line it is on.

use std::collections::HashMap;
use std::rc::Rc;

use super::lex::{lex, TemplatePiece, Tok, Token};

/// A name, as its index in [`Program::names`]. Comparing two names is then
/// comparing two numbers, which is what a variable lookup does all day.
pub(super) type Sym = u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Unary {
    Not,
    Neg,
    Plus,
    BitNot,
    TypeOf,
    Void,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Binary {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Eq,
    Ne,
    StrictEq,
    StrictNe,
    Lt,
    Le,
    Gt,
    Ge,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    UShr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Logical {
    And,
    Or,
    Coalesce,
}

#[derive(Debug)]
pub(super) enum Expr {
    Num(f64),
    Str(Rc<str>),
    Bool(bool),
    Null,
    Ident(Sym),
    Array(Vec<Expr>),
    Object(Vec<(Rc<str>, Expr)>),
    /// Text and holes, in order.
    Template(Vec<Result<Rc<str>, Expr>>),
    Unary(Unary, Box<Expr>),
    /// `++x`, `x--`: the amount, and whether the old value is the answer.
    Update {
        target: Box<Expr>,
        delta: f64,
        prefix: bool,
    },
    Binary(Binary, Box<Expr>, Box<Expr>),
    Logical(Logical, Box<Expr>, Box<Expr>),
    Cond(Box<Expr>, Box<Expr>, Box<Expr>),
    Assign {
        target: Box<Expr>,
        op: Option<Binary>,
        logical: Option<Logical>,
        value: Box<Expr>,
    },
    Member {
        object: Box<Expr>,
        name: Rc<str>,
        optional: bool,
    },
    Index(Box<Expr>, Box<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    Function(Rc<Function>),
    Seq(Vec<Expr>),
}

#[derive(Debug)]
pub(super) struct Function {
    pub name: Option<Sym>,
    pub params: Vec<Sym>,
    pub body: Vec<Stmt>,
}

/// What a declaration binds: one name, or the items of an array by position.
#[derive(Debug)]
pub(super) enum Pattern {
    Name(Sym),
    Array(Vec<Option<Sym>>),
}

#[derive(Debug)]
pub(super) enum Stmt {
    Expr(Expr),
    Var {
        /// `let` and `const` belong to their block, `var` to its function.
        block_scoped: bool,
        decls: Vec<(Pattern, Option<Expr>)>,
    },
    Function(Rc<Function>),
    Return(Option<Expr>),
    If(Expr, Box<Stmt>, Option<Box<Stmt>>),
    For {
        init: Option<Box<Stmt>>,
        test: Option<Expr>,
        update: Option<Expr>,
        body: Box<Stmt>,
    },
    /// `for (x of list)` and, with `keys`, `for (x in list)`.
    ForEach {
        name: Sym,
        keys: bool,
        subject: Expr,
        body: Box<Stmt>,
    },
    While(Expr, Box<Stmt>),
    DoWhile(Box<Stmt>, Expr),
    Block(Vec<Stmt>),
    Break,
    Continue,
    Throw(Expr),
    Try {
        body: Vec<Stmt>,
        param: Option<Sym>,
        handler: Option<Vec<Stmt>>,
        finally: Option<Vec<Stmt>>,
    },
    Switch {
        subject: Expr,
        cases: Vec<(Option<Expr>, Vec<Stmt>)>,
    },
    Empty,
}

#[derive(Debug)]
pub(crate) struct Program {
    pub(super) body: Vec<Stmt>,
    pub(super) names: Vec<Rc<str>>,
    /// Every name the text reads without declaring it — `time`, `wiggle`,
    /// `thisComp`. Whether Lumit knows them all is how an expression is
    /// judged runnable before it is run.
    pub(super) free: Vec<Sym>,
}

impl Program {
    pub(super) fn name(&self, sym: Sym) -> &str {
        self.names.get(sym as usize).map_or("", |n| n)
    }

    /// The names the text reads without declaring.
    pub(crate) fn free_names(&self) -> impl Iterator<Item = &str> {
        self.free.iter().map(|s| self.name(*s))
    }
}

/// How deep the tree may go: brackets, blocks and functions as they nest, and
/// each link of a chain such as `a + b + c` or `a.b.c`, which hangs one under
/// the other. The parser calls itself once per level and so does everything
/// that walks the tree afterwards, so without a ceiling a line of ten thousand
/// opening brackets, or of ten thousand `+ 1`, is a stack overflow rather than
/// a refusal.
const MAX_DEPTH: u32 = 120;

#[derive(Default)]
struct Names {
    list: Vec<Rc<str>>,
    index: HashMap<Rc<str>, Sym>,
    read: Vec<Sym>,
    declared: Vec<Sym>,
}

impl Names {
    fn sym(&mut self, name: &str) -> Sym {
        if let Some(sym) = self.index.get(name) {
            return *sym;
        }
        let sym = self.list.len() as Sym;
        let name: Rc<str> = Rc::from(name);
        self.list.push(name.clone());
        self.index.insert(name, sym);
        sym
    }
}

struct Parser<'n> {
    tokens: Vec<Token>,
    at: usize,
    depth: u32,
    names: &'n mut Names,
}

pub(super) fn parse(source: &str) -> Result<Program, String> {
    let mut names = Names::default();
    let body = Parser::new(source, &mut names, 0)?.program()?;
    names.read.sort_unstable();
    names.read.dedup();
    let free = names
        .read
        .iter()
        .copied()
        .filter(|s| !names.declared.contains(s))
        .collect();
    Ok(Program {
        body,
        names: names.list,
        free,
    })
}

const ASSIGN_OPS: [(&str, Option<Binary>, Option<Logical>); 16] = [
    ("=", None, None),
    ("+=", Some(Binary::Add), None),
    ("-=", Some(Binary::Sub), None),
    ("*=", Some(Binary::Mul), None),
    ("/=", Some(Binary::Div), None),
    ("%=", Some(Binary::Rem), None),
    ("**=", Some(Binary::Pow), None),
    ("&=", Some(Binary::BitAnd), None),
    ("|=", Some(Binary::BitOr), None),
    ("^=", Some(Binary::BitXor), None),
    ("<<=", Some(Binary::Shl), None),
    (">>=", Some(Binary::Shr), None),
    (">>>=", Some(Binary::UShr), None),
    ("&&=", None, Some(Logical::And)),
    ("||=", None, Some(Logical::Or)),
    ("??=", None, Some(Logical::Coalesce)),
];

/// A binary operator's strength, higher binding tighter.
fn binary_op(p: &str) -> Option<(u8, Result<Binary, Logical>)> {
    Some(match p {
        "??" => (1, Err(Logical::Coalesce)),
        "||" => (2, Err(Logical::Or)),
        "&&" => (3, Err(Logical::And)),
        "|" => (4, Ok(Binary::BitOr)),
        "^" => (5, Ok(Binary::BitXor)),
        "&" => (6, Ok(Binary::BitAnd)),
        "==" => (7, Ok(Binary::Eq)),
        "!=" => (7, Ok(Binary::Ne)),
        "===" => (7, Ok(Binary::StrictEq)),
        "!==" => (7, Ok(Binary::StrictNe)),
        "<" => (8, Ok(Binary::Lt)),
        "<=" => (8, Ok(Binary::Le)),
        ">" => (8, Ok(Binary::Gt)),
        ">=" => (8, Ok(Binary::Ge)),
        "<<" => (9, Ok(Binary::Shl)),
        ">>" => (9, Ok(Binary::Shr)),
        ">>>" => (9, Ok(Binary::UShr)),
        "+" => (10, Ok(Binary::Add)),
        "-" => (10, Ok(Binary::Sub)),
        "*" => (11, Ok(Binary::Mul)),
        "/" => (11, Ok(Binary::Div)),
        "%" => (11, Ok(Binary::Rem)),
        "**" => (12, Ok(Binary::Pow)),
        _ => return None,
    })
}

impl<'n> Parser<'n> {
    fn new(source: &str, names: &'n mut Names, depth: u32) -> Result<Self, String> {
        Ok(Parser {
            tokens: lex(source)?,
            at: 0,
            depth,
            names,
        })
    }

    fn peek(&self) -> &Tok {
        self.peek_at(0)
    }

    fn peek_at(&self, ahead: usize) -> &Tok {
        self.tokens
            .get(self.at + ahead)
            .map_or(&Tok::Eof, |t| &t.tok)
    }

    fn line(&self) -> u32 {
        self.tokens
            .get(self.at)
            .or(self.tokens.last())
            .map_or(1, |t| t.line)
    }

    fn newline_before(&self) -> bool {
        self.tokens.get(self.at).is_none_or(|t| t.newline_before)
    }

    fn next(&mut self) -> Tok {
        let tok = self.peek().clone();
        if self.at < self.tokens.len() {
            self.at += 1;
        }
        tok
    }

    fn is(&self, p: &str) -> bool {
        matches!(self.peek(), Tok::Punct(q) if *q == p)
    }

    fn eat(&mut self, p: &str) -> bool {
        let hit = self.is(p);
        if hit {
            self.at += 1;
        }
        hit
    }

    fn is_word(&self, word: &str) -> bool {
        matches!(self.peek(), Tok::Ident(w) if w == word)
    }

    fn eat_word(&mut self, word: &str) -> bool {
        let hit = self.is_word(word);
        if hit {
            self.at += 1;
        }
        hit
    }

    fn fail<T>(&self, what: &str) -> Result<T, String> {
        Err(format!("line {}: {what}", self.line()))
    }

    fn expect(&mut self, p: &str) -> Result<(), String> {
        if self.eat(p) {
            Ok(())
        } else {
            self.fail(&format!("expected '{p}'"))
        }
    }

    fn ident(&mut self) -> Result<Sym, String> {
        match self.next() {
            Tok::Ident(name) => Ok(self.names.sym(&name)),
            _ => {
                self.at = self.at.saturating_sub(1);
                self.fail("expected a name")
            }
        }
    }

    fn declare(&mut self) -> Result<Sym, String> {
        let sym = self.ident()?;
        self.names.declared.push(sym);
        Ok(sym)
    }

    fn deeper(&mut self) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return self.fail("the expression nests too deeply");
        }
        Ok(())
    }

    /// A statement ends at a semicolon, at a closing brace, at the end of the
    /// text, or at a line break — which is how most expressions are written.
    fn end_statement(&mut self) -> Result<(), String> {
        if self.eat(";") || self.is("}") || *self.peek() == Tok::Eof || self.newline_before() {
            Ok(())
        } else {
            self.fail("expected ';'")
        }
    }

    fn program(&mut self) -> Result<Vec<Stmt>, String> {
        let mut body = Vec::new();
        while *self.peek() != Tok::Eof {
            body.push(self.statement()?);
        }
        Ok(body)
    }

    fn block(&mut self) -> Result<Vec<Stmt>, String> {
        self.expect("{")?;
        let mut body = Vec::new();
        while !self.is("}") {
            if *self.peek() == Tok::Eof {
                return self.fail("a block that never closes");
            }
            body.push(self.statement()?);
        }
        self.expect("}")?;
        Ok(body)
    }

    fn statement(&mut self) -> Result<Stmt, String> {
        self.deeper()?;
        let stmt = self.statement_inner();
        self.depth -= 1;
        stmt
    }

    fn statement_inner(&mut self) -> Result<Stmt, String> {
        if self.eat(";") {
            return Ok(Stmt::Empty);
        }
        if self.is("{") {
            return Ok(Stmt::Block(self.block()?));
        }
        let Tok::Ident(word) = self.peek().clone() else {
            return self.expression_statement();
        };
        // A keyword followed by `.` or `=` is somebody's variable.
        let keyword = !matches!(self.peek_at(1), Tok::Punct("." | "=" | ","));
        match word.as_str() {
            "var" | "let" | "const" if keyword => {
                self.at += 1;
                let stmt = self.declaration(word != "var")?;
                self.end_statement()?;
                Ok(stmt)
            }
            "function" if matches!(self.peek_at(1), Tok::Ident(_)) => {
                self.at += 1;
                let name = self.declare()?;
                Ok(Stmt::Function(self.function_rest(Some(name))?))
            }
            "return" if keyword => {
                self.at += 1;
                let value = if self.is(";")
                    || self.is("}")
                    || *self.peek() == Tok::Eof
                    || self.newline_before()
                {
                    None
                } else {
                    Some(self.expression()?)
                };
                self.end_statement()?;
                Ok(Stmt::Return(value))
            }
            "if" if self.is_at(1, "(") => {
                self.at += 1;
                self.expect("(")?;
                let test = self.expression()?;
                self.expect(")")?;
                let then = Box::new(self.statement()?);
                let other = if self.eat_word("else") {
                    Some(Box::new(self.statement()?))
                } else {
                    None
                };
                Ok(Stmt::If(test, then, other))
            }
            "for" if self.is_at(1, "(") => {
                self.at += 1;
                self.for_statement()
            }
            "while" if self.is_at(1, "(") => {
                self.at += 1;
                self.expect("(")?;
                let test = self.expression()?;
                self.expect(")")?;
                Ok(Stmt::While(test, Box::new(self.statement()?)))
            }
            "do" if self.is_at(1, "{") => {
                self.at += 1;
                let body = Box::new(self.statement()?);
                if !self.eat_word("while") {
                    return self.fail("expected 'while'");
                }
                self.expect("(")?;
                let test = self.expression()?;
                self.expect(")")?;
                self.eat(";");
                Ok(Stmt::DoWhile(body, test))
            }
            "break" | "continue" if keyword => {
                self.at += 1;
                self.end_statement()?;
                Ok(if word == "break" {
                    Stmt::Break
                } else {
                    Stmt::Continue
                })
            }
            "throw" if keyword => {
                self.at += 1;
                let value = self.expression()?;
                self.end_statement()?;
                Ok(Stmt::Throw(value))
            }
            "try" if self.is_at(1, "{") => {
                self.at += 1;
                self.try_statement()
            }
            "switch" if self.is_at(1, "(") => {
                self.at += 1;
                self.switch_statement()
            }
            _ => self.expression_statement(),
        }
    }

    fn is_at(&self, ahead: usize, p: &str) -> bool {
        matches!(self.peek_at(ahead), Tok::Punct(q) if *q == p)
    }

    fn expression_statement(&mut self) -> Result<Stmt, String> {
        let expr = self.expression()?;
        self.end_statement()?;
        Ok(Stmt::Expr(expr))
    }

    fn declaration(&mut self, block_scoped: bool) -> Result<Stmt, String> {
        let mut decls = Vec::new();
        loop {
            let pattern = if self.eat("[") {
                let mut items = Vec::new();
                while !self.eat("]") {
                    if self.is(",") {
                        items.push(None);
                    } else {
                        items.push(Some(self.declare()?));
                    }
                    if !self.is("]") {
                        self.expect(",")?;
                    }
                }
                Pattern::Array(items)
            } else {
                Pattern::Name(self.declare()?)
            };
            let init = if self.eat("=") {
                Some(self.assignment()?)
            } else {
                None
            };
            decls.push((pattern, init));
            if !self.eat(",") {
                break;
            }
        }
        Ok(Stmt::Var {
            block_scoped,
            decls,
        })
    }

    fn for_statement(&mut self) -> Result<Stmt, String> {
        self.expect("(")?;
        // `for (var x of list)` and `for (x in list)`.
        let declares =
            matches!(self.peek(), Tok::Ident(w) if matches!(w.as_str(), "var" | "let" | "const"));
        let name_at = usize::from(declares);
        if let (Tok::Ident(_), Tok::Ident(word)) =
            (self.peek_at(name_at), self.peek_at(name_at + 1).clone())
        {
            if word == "of" || word == "in" {
                self.at += name_at;
                let name = self.declare()?;
                self.at += 1;
                let subject = self.expression()?;
                self.expect(")")?;
                return Ok(Stmt::ForEach {
                    name,
                    keys: word == "in",
                    subject,
                    body: Box::new(self.statement()?),
                });
            }
        }
        let init = if self.eat(";") {
            None
        } else {
            let init = if declares {
                let block_scoped = !self.is_word("var");
                self.at += 1;
                self.declaration(block_scoped)?
            } else {
                Stmt::Expr(self.expression()?)
            };
            self.expect(";")?;
            Some(Box::new(init))
        };
        let test = if self.is(";") {
            None
        } else {
            Some(self.expression()?)
        };
        self.expect(";")?;
        let update = if self.is(")") {
            None
        } else {
            Some(self.expression()?)
        };
        self.expect(")")?;
        Ok(Stmt::For {
            init,
            test,
            update,
            body: Box::new(self.statement()?),
        })
    }

    fn try_statement(&mut self) -> Result<Stmt, String> {
        let body = self.block()?;
        let mut param = None;
        let handler = if self.eat_word("catch") {
            if self.eat("(") {
                param = Some(self.declare()?);
                self.expect(")")?;
            }
            Some(self.block()?)
        } else {
            None
        };
        let finally = if self.eat_word("finally") {
            Some(self.block()?)
        } else {
            None
        };
        if handler.is_none() && finally.is_none() {
            return self.fail("expected 'catch'");
        }
        Ok(Stmt::Try {
            body,
            param,
            handler,
            finally,
        })
    }

    fn switch_statement(&mut self) -> Result<Stmt, String> {
        self.expect("(")?;
        let subject = self.expression()?;
        self.expect(")")?;
        self.expect("{")?;
        let mut cases: Vec<(Option<Expr>, Vec<Stmt>)> = Vec::new();
        while !self.eat("}") {
            let test = if self.eat_word("default") {
                None
            } else if self.eat_word("case") {
                Some(self.expression()?)
            } else {
                return self.fail("expected 'case'");
            };
            self.expect(":")?;
            let mut body = Vec::new();
            while !self.is("}") && !self.is_word("case") && !self.is_word("default") {
                if *self.peek() == Tok::Eof {
                    return self.fail("a block that never closes");
                }
                body.push(self.statement()?);
            }
            cases.push((test, body));
        }
        Ok(Stmt::Switch { subject, cases })
    }

    /// The parameters and body of a function, after its name.
    fn function_rest(&mut self, name: Option<Sym>) -> Result<Rc<Function>, String> {
        self.expect("(")?;
        let mut params = Vec::new();
        while !self.eat(")") {
            params.push(self.declare()?);
            // A default value is read and dropped: a missing argument is
            // `undefined`, which the body's own arithmetic then sees.
            if self.eat("=") {
                self.assignment()?;
            }
            if !self.is(")") {
                self.expect(",")?;
            }
        }
        let body = self.block()?;
        Ok(Rc::new(Function { name, params, body }))
    }

    fn expression(&mut self) -> Result<Expr, String> {
        let first = self.assignment()?;
        if !self.is(",") {
            return Ok(first);
        }
        let mut all = vec![first];
        while self.eat(",") {
            all.push(self.assignment()?);
        }
        Ok(Expr::Seq(all))
    }

    /// Whether the tokens from here are an arrow function's parameters:
    /// `x =>` or `(a, b) =>`.
    fn arrow_ahead(&self) -> bool {
        match self.peek() {
            Tok::Ident(_) => self.is_at(1, "=>"),
            Tok::Punct("(") => {
                let mut ahead = 1;
                loop {
                    match self.peek_at(ahead) {
                        Tok::Punct(")") => return self.is_at(ahead + 1, "=>"),
                        Tok::Ident(_) | Tok::Punct(",") => ahead += 1,
                        _ => return false,
                    }
                }
            }
            _ => false,
        }
    }

    fn arrow(&mut self) -> Result<Expr, String> {
        let mut params = Vec::new();
        if self.eat("(") {
            while !self.eat(")") {
                params.push(self.declare()?);
                self.eat(",");
            }
        } else {
            params.push(self.declare()?);
        }
        self.expect("=>")?;
        let body = if self.is("{") {
            self.block()?
        } else {
            vec![Stmt::Return(Some(self.assignment()?))]
        };
        Ok(Expr::Function(Rc::new(Function {
            name: None,
            params,
            body,
        })))
    }

    fn assignment(&mut self) -> Result<Expr, String> {
        self.deeper()?;
        let expr = self.assignment_inner();
        self.depth -= 1;
        expr
    }

    fn assignment_inner(&mut self) -> Result<Expr, String> {
        if self.arrow_ahead() {
            return self.arrow();
        }
        let target = self.conditional()?;
        let Tok::Punct(p) = *self.peek() else {
            return Ok(target);
        };
        let Some((_, op, logical)) = ASSIGN_OPS.iter().find(|(text, ..)| *text == p) else {
            return Ok(target);
        };
        match &target {
            Expr::Ident(sym) => {
                // Assigning to a name nobody declared makes it, as it does in
                // After Effects, so `x = 5` on its own line is a declaration.
                if op.is_none() && logical.is_none() {
                    self.names.declared.push(*sym);
                }
            }
            Expr::Member { .. } | Expr::Index(..) => {}
            _ => return self.fail("this cannot be assigned to"),
        }
        self.at += 1;
        let value = self.assignment()?;
        Ok(Expr::Assign {
            target: Box::new(target),
            op: *op,
            logical: *logical,
            value: Box::new(value),
        })
    }

    fn conditional(&mut self) -> Result<Expr, String> {
        let test = self.binary(0)?;
        if !self.eat("?") {
            return Ok(test);
        }
        let then = self.assignment()?;
        self.expect(":")?;
        let other = self.assignment()?;
        Ok(Expr::Cond(Box::new(test), Box::new(then), Box::new(other)))
    }

    fn binary(&mut self, floor: u8) -> Result<Expr, String> {
        let held = self.depth;
        let expr = self.binary_inner(floor);
        self.depth = held;
        expr
    }

    fn binary_inner(&mut self, floor: u8) -> Result<Expr, String> {
        let mut left = self.unary()?;
        loop {
            let Tok::Punct(p) = *self.peek() else {
                return Ok(left);
            };
            let Some((strength, op)) = binary_op(p) else {
                return Ok(left);
            };
            if strength <= floor {
                return Ok(left);
            }
            self.at += 1;
            self.deeper()?;
            // `**` groups to the right, everything else to the left.
            let right = self.binary(if p == "**" { strength - 1 } else { strength })?;
            left = match op {
                Ok(op) => Expr::Binary(op, Box::new(left), Box::new(right)),
                Err(op) => Expr::Logical(op, Box::new(left), Box::new(right)),
            };
        }
    }

    fn unary(&mut self) -> Result<Expr, String> {
        self.deeper()?;
        let expr = self.unary_inner();
        self.depth -= 1;
        expr
    }

    fn unary_inner(&mut self) -> Result<Expr, String> {
        let op = match self.peek() {
            Tok::Punct("!") => Some(Unary::Not),
            Tok::Punct("-") => Some(Unary::Neg),
            Tok::Punct("+") => Some(Unary::Plus),
            Tok::Punct("~") => Some(Unary::BitNot),
            Tok::Ident(w) if w == "typeof" => Some(Unary::TypeOf),
            Tok::Ident(w) if w == "void" => Some(Unary::Void),
            _ => None,
        };
        if let Some(op) = op {
            self.at += 1;
            return Ok(Expr::Unary(op, Box::new(self.unary()?)));
        }
        for (p, delta) in [("++", 1.0), ("--", -1.0)] {
            if self.eat(p) {
                let target = self.unary()?;
                return self.update(target, delta, true);
            }
        }
        let mut expr = self.postfix()?;
        for (p, delta) in [("++", 1.0), ("--", -1.0)] {
            if self.is(p) && !self.newline_before() {
                self.at += 1;
                expr = self.update(expr, delta, false)?;
            }
        }
        Ok(expr)
    }

    fn update(&self, target: Expr, delta: f64, prefix: bool) -> Result<Expr, String> {
        if !matches!(
            target,
            Expr::Ident(_) | Expr::Member { .. } | Expr::Index(..)
        ) {
            return self.fail("this cannot be assigned to");
        }
        Ok(Expr::Update {
            target: Box::new(target),
            delta,
            prefix,
        })
    }

    fn arguments(&mut self) -> Result<Vec<Expr>, String> {
        let mut args = Vec::new();
        while !self.eat(")") {
            if *self.peek() == Tok::Eof {
                return self.fail("a bracket that never closes");
            }
            args.push(self.assignment()?);
            if !self.is(")") {
                self.expect(",")?;
            }
        }
        Ok(args)
    }

    fn member_name(&mut self) -> Result<Rc<str>, String> {
        match self.next() {
            Tok::Ident(name) => Ok(Rc::from(name.as_str())),
            _ => {
                self.at = self.at.saturating_sub(1);
                self.fail("expected a name")
            }
        }
    }

    fn postfix(&mut self) -> Result<Expr, String> {
        let held = self.depth;
        let expr = self.postfix_inner();
        self.depth = held;
        expr
    }

    fn postfix_inner(&mut self) -> Result<Expr, String> {
        let mut expr = self.primary()?;
        loop {
            self.deeper()?;
            if self.eat(".") {
                expr = Expr::Member {
                    object: Box::new(expr),
                    name: self.member_name()?,
                    optional: false,
                };
            } else if self.eat("?.") {
                expr = Expr::Member {
                    object: Box::new(expr),
                    name: self.member_name()?,
                    optional: true,
                };
            } else if self.eat("[") {
                let index = self.expression()?;
                self.expect("]")?;
                expr = Expr::Index(Box::new(expr), Box::new(index));
            } else if self.eat("(") {
                // A `(` at the start of a line still belongs to the line
                // above, as JavaScript has it. That is what makes
                // `effect("Shake")` then `("Seed")` on the next line one call.
                expr = Expr::Call(Box::new(expr), self.arguments()?);
            } else {
                return Ok(expr);
            }
        }
    }

    fn primary(&mut self) -> Result<Expr, String> {
        match self.next() {
            Tok::Num(n) => Ok(Expr::Num(n)),
            Tok::Str(s) => Ok(Expr::Str(Rc::from(s.as_str()))),
            Tok::Template(pieces) => {
                let mut out = Vec::new();
                for piece in pieces {
                    out.push(match piece {
                        TemplatePiece::Text(text) => Ok(Rc::from(text.as_str())),
                        TemplatePiece::Code(code) => {
                            let mut inner = Parser::new(&code, self.names, self.depth + 1)?;
                            inner.deeper()?;
                            let expr = inner.expression()?;
                            if *inner.peek() != Tok::Eof {
                                return self.fail("a template hole holds one expression");
                            }
                            Err(expr)
                        }
                    });
                }
                Ok(Expr::Template(out))
            }
            Tok::Punct("(") => {
                let expr = self.expression()?;
                self.expect(")")?;
                Ok(expr)
            }
            Tok::Punct("[") => {
                let mut items = Vec::new();
                while !self.eat("]") {
                    if *self.peek() == Tok::Eof {
                        return self.fail("a bracket that never closes");
                    }
                    items.push(self.assignment()?);
                    if !self.is("]") {
                        self.expect(",")?;
                    }
                }
                Ok(Expr::Array(items))
            }
            Tok::Punct("{") => {
                let mut fields = Vec::new();
                while !self.eat("}") {
                    let key: Rc<str> = match self.next() {
                        Tok::Ident(name) => Rc::from(name.as_str()),
                        Tok::Str(name) => Rc::from(name.as_str()),
                        Tok::Num(n) => Rc::from(super::interp::number_text(n).as_str()),
                        _ => {
                            self.at = self.at.saturating_sub(1);
                            return self.fail("expected a name");
                        }
                    };
                    let value = if self.eat(":") {
                        self.assignment()?
                    } else {
                        // `{x, y}` is `{x: x, y: y}`.
                        let sym = self.names.sym(&key);
                        self.names.read.push(sym);
                        Expr::Ident(sym)
                    };
                    fields.push((key, value));
                    if !self.is("}") {
                        self.expect(",")?;
                    }
                }
                Ok(Expr::Object(fields))
            }
            Tok::Ident(word) => match word.as_str() {
                "true" => Ok(Expr::Bool(true)),
                "false" => Ok(Expr::Bool(false)),
                "null" => Ok(Expr::Null),
                "function" => {
                    let name = if matches!(self.peek(), Tok::Ident(_)) {
                        Some(self.declare()?)
                    } else {
                        None
                    };
                    Ok(Expr::Function(self.function_rest(name)?))
                }
                "new" | "class" | "delete" | "await" | "yield" | "import" => {
                    self.at = self.at.saturating_sub(1);
                    self.fail(&format!("'{word}' is not available in an expression"))
                }
                _ => {
                    let sym = self.names.sym(&word);
                    self.names.read.push(sym);
                    Ok(Expr::Ident(sym))
                }
            },
            Tok::Eof => self.fail("the expression stops short"),
            Tok::Punct(p) => {
                self.at = self.at.saturating_sub(1);
                self.fail(&format!("unexpected '{p}'"))
            }
        }
    }
}
