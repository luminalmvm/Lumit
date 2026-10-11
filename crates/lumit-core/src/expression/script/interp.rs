//! Walking the tree: values, variables, arithmetic and the parts of
//! JavaScript's own library an expression leans on (arrays, text, `Math`).
//!
//! What After Effects adds on top (`thisComp`, `effect(…)`, `wiggle`) is in
//! [`super::ae`]. The two meet at [`Host`]: a value this file carries about
//! without looking inside.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use super::ae::{Ae, Host};
use super::parse::{Binary, Expr, Function, Logical, Pattern, Program, Stmt, Sym, Unary};

// What one evaluation may spend. A property is read for every frame, twice,
// so an expression that loops for ever or builds something enormous has to
// stop on its own: there is nobody watching a render thread.
//
// A step is roughly one thing done. Work that is bigger than that is charged
// by its size ([`Interp::spend`]): a list by its items, text by its length. So
// the one budget bounds the time a run takes and the memory it holds as well.
const MAX_STEPS: u32 = 2_000_000;
const MAX_CALL_DEPTH: u32 = 64;
pub(super) const MAX_ITEMS: usize = 16_384;
const MAX_TEXT_BYTES: usize = 1 << 20;
/// How far a list inside a list is followed when it is read as text or as a
/// number. A list can hold itself, so this is also what stops that going
/// round for ever.
const MAX_LIST_DEPTH: u32 = 16;

thread_local! {
    // The budget belongs to the whole evaluation, not to one interpreter. An
    // expression that reads another expression's property starts a second
    // interpreter inside the first, and a budget each would multiply: twenty
    // layers that each follow the one before would be a million full runs.
    static RUNNING: Cell<u32> = const { Cell::new(0) };
    static SPENT: Cell<u32> = const { Cell::new(0) };
    static LIMIT: Cell<u32> = const { Cell::new(MAX_STEPS) };
}

/// Held for as long as an evaluation runs on this thread. The first one in
/// starts the count again, the ones inside it carry on from where it is.
pub(super) struct Running;

impl Running {
    pub(super) fn begin() -> Running {
        RUNNING.with(|running| {
            if running.get() == 0 {
                SPENT.with(|spent| spent.set(0));
            }
            running.set(running.get() + 1);
        });
        Running
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        RUNNING.with(|running| running.set(running.get().saturating_sub(1)));
    }
}

/// Run `work` with every evaluation inside it sharing one budget of `steps`.
/// For the interface's own thread, which must never wait as long as a render
/// may.
pub(crate) fn sparingly<T>(steps: u32, work: impl FnOnce() -> T) -> T {
    let before = LIMIT.with(|limit| limit.replace(steps.min(MAX_STEPS)));
    let running = Running::begin();
    let done = work();
    drop(running);
    LIMIT.with(|limit| limit.set(before));
    done
}

fn spent_out() -> bool {
    SPENT.with(Cell::get) > LIMIT.with(Cell::get)
}

#[derive(Clone)]
pub(super) enum Value {
    Undefined,
    Null,
    Bool(bool),
    Num(f64),
    Str(Rc<str>),
    Array(Rc<RefCell<Vec<Value>>>),
    Object(Rc<RefCell<Fields>>),
    Func(Rc<Closure>),
    /// A built-in, as the thing it was reached on and its name: `Math.floor`,
    /// `list.push`, and `wiggle` on its own (reached on the global object).
    Bound(Rc<(Value, Rc<str>)>),
    Host(Host),
}

/// An object's fields, in the order they were written.
pub(super) type Fields = Vec<(Rc<str>, Value)>;

pub(super) struct Closure {
    func: Rc<Function>,
    env: Rc<Env>,
}

/// Why an evaluation stopped early.
pub(super) enum Abort {
    Error(String),
    Throw(Value),
}

pub(super) type Res<T> = Result<T, Abort>;

pub(super) fn fail<T>(why: impl Into<String>) -> Res<T> {
    Err(Abort::Error(why.into()))
}

pub(super) struct Env {
    vars: RefCell<Vec<(Sym, Value)>>,
    parent: Option<Rc<Env>>,
    /// Whether this is a function's own scope (or the program's), which is
    /// where a `var` lands however deep the block it was written in.
    function: bool,
}

impl Env {
    fn new(parent: Option<Rc<Env>>, function: bool) -> Rc<Env> {
        Rc::new(Env {
            vars: RefCell::new(Vec::new()),
            parent,
            function,
        })
    }

    fn get(&self, sym: Sym) -> Option<Value> {
        let mut env = self;
        loop {
            if let Some((_, value)) = env.vars.borrow().iter().find(|(s, _)| *s == sym) {
                return Some(value.clone());
            }
            env = env.parent.as_deref()?;
        }
    }

    /// Write to a name that exists somewhere up the chain. `false` when none
    /// does.
    fn set(&self, sym: Sym, value: &Value) -> bool {
        let mut env = self;
        loop {
            if let Some((_, slot)) = env.vars.borrow_mut().iter_mut().find(|(s, _)| *s == sym) {
                *slot = value.clone();
                return true;
            }
            match env.parent.as_deref() {
                Some(parent) => env = parent,
                None => return false,
            }
        }
    }

    fn define(&self, sym: Sym, value: Value) {
        let mut vars = self.vars.borrow_mut();
        match vars.iter_mut().find(|(s, _)| *s == sym) {
            Some((_, slot)) => *slot = value,
            None => vars.push((sym, value)),
        }
    }

    fn function_scope(self: &Rc<Env>) -> Rc<Env> {
        let mut env = self.clone();
        while !env.function {
            match &env.parent {
                Some(parent) => env = parent.clone(),
                None => break,
            }
        }
        env
    }
}

enum Flow {
    Next,
    Break,
    Continue,
    Return(Value),
}

pub(super) struct Interp<'a> {
    program: &'a Program,
    pub(super) ae: Ae<'a>,
    calls: u32,
    /// The value of the last expression statement that ran, which is what an
    /// expression answers with.
    last: Value,
    root: Rc<Env>,
    /// Everything made during the run that can hold a reference to something
    /// else. A list that holds itself, or a function that closes over the
    /// scope it lives in, would otherwise never be freed, and this runs
    /// tens of thousands of times a second. They are emptied when the run ends.
    heap: Vec<Value>,
    scopes: Vec<Rc<Env>>,
}

impl Drop for Interp<'_> {
    fn drop(&mut self) {
        for value in self.heap.drain(..) {
            match value {
                Value::Array(items) => items.borrow_mut().clear(),
                Value::Object(fields) => fields.borrow_mut().clear(),
                _ => {}
            }
        }
        for env in self.scopes.drain(..) {
            env.vars.borrow_mut().clear();
        }
    }
}

impl<'a> Interp<'a> {
    pub(super) fn new(program: &'a Program, ae: Ae<'a>) -> Self {
        let root = Env::new(None, true);
        Interp {
            program,
            ae,
            calls: 0,
            last: Value::Undefined,
            scopes: vec![root.clone()],
            root,
            heap: Vec::new(),
        }
    }

    /// Run the program and answer with what its last statement came to.
    pub(super) fn run(&mut self) -> Result<Value, String> {
        let program = self.program;
        let root = self.root.clone();
        let done = self.block(&program.body, &root);
        match done {
            Ok(Flow::Return(value)) => Ok(value),
            Ok(_) => Ok(std::mem::replace(&mut self.last, Value::Undefined)),
            Err(Abort::Error(why)) => Err(why),
            Err(Abort::Throw(value)) => Err(match self.plain(value) {
                Ok(value) => self.text(&value),
                Err(_) => "the expression threw".into(),
            }),
        }
    }

    fn step(&mut self) -> Res<()> {
        self.spend(1)
    }

    /// Charge `work` steps at once: for something whose cost is its size.
    fn spend(&mut self, work: usize) -> Res<()> {
        let work = u32::try_from(work).unwrap_or(u32::MAX);
        SPENT.with(|spent| spent.set(spent.get().saturating_add(work)));
        if spent_out() {
            return fail("the expression ran for too long");
        }
        Ok(())
    }

    pub(super) fn array(&mut self, items: Vec<Value>) -> Res<Value> {
        if items.len() > MAX_ITEMS {
            return fail("a list too long to hold");
        }
        // Every list made is kept until the run ends, so each is paid for.
        self.spend(items.len())?;
        let value = Value::Array(Rc::new(RefCell::new(items)));
        self.heap.push(value.clone());
        Ok(value)
    }

    pub(super) fn numbers(&mut self, items: &[f64]) -> Res<Value> {
        self.array(items.iter().map(|n| Value::Num(*n)).collect())
    }

    pub(super) fn object(&mut self, fields: Fields) -> Value {
        let value = Value::Object(Rc::new(RefCell::new(fields)));
        self.heap.push(value.clone());
        value
    }

    fn string(&mut self, text: String) -> Res<Value> {
        if text.len() > MAX_TEXT_BYTES {
            return fail("text too long to hold");
        }
        self.spend(text.len() / 16)?;
        Ok(Value::Str(Rc::from(text.as_str())))
    }

    // ───────────────────────────── statements ─────────────────────────────

    fn block(&mut self, body: &[Stmt], env: &Rc<Env>) -> Res<Flow> {
        // A function can be called above the line it is written on.
        for stmt in body {
            if let Stmt::Function(func) = stmt {
                let closure = self.closure(func, env);
                if let Some(name) = func.name {
                    env.define(name, closure);
                }
            }
        }
        for stmt in body {
            match self.exec(stmt, env)? {
                Flow::Next => {}
                other => return Ok(other),
            }
        }
        Ok(Flow::Next)
    }

    fn scoped(&mut self, body: &[Stmt], env: &Rc<Env>) -> Res<Flow> {
        let own = body.iter().any(|s| {
            matches!(
                s,
                Stmt::Function(_)
                    | Stmt::Var {
                        block_scoped: true,
                        ..
                    }
            )
        });
        if own {
            let inner = Env::new(Some(env.clone()), false);
            self.block(body, &inner)
        } else {
            self.block(body, env)
        }
    }

    fn closure(&mut self, func: &Rc<Function>, env: &Rc<Env>) -> Value {
        self.scopes.push(env.clone());
        Value::Func(Rc::new(Closure {
            func: func.clone(),
            env: env.clone(),
        }))
    }

    fn exec(&mut self, stmt: &Stmt, env: &Rc<Env>) -> Res<Flow> {
        self.step()?;
        match stmt {
            Stmt::Empty | Stmt::Function(_) => {}
            Stmt::Expr(expr) => self.last = self.eval(expr, env)?,
            Stmt::Var {
                block_scoped,
                decls,
            } => {
                let scope = if *block_scoped {
                    env.clone()
                } else {
                    env.function_scope()
                };
                for (pattern, init) in decls {
                    let value = match init {
                        Some(init) => Some(self.eval(init, env)?),
                        None => None,
                    };
                    match pattern {
                        Pattern::Name(name) => match value {
                            Some(value) => scope.define(*name, value),
                            // `var x;` over an `x` that already holds
                            // something leaves it alone.
                            None if scope.get(*name).is_none() || *block_scoped => {
                                scope.define(*name, Value::Undefined);
                            }
                            None => {}
                        },
                        Pattern::Array(names) => {
                            let value = self.plain(value.unwrap_or(Value::Undefined))?;
                            let Value::Array(items) = value else {
                                return fail("only a list can be unpacked");
                            };
                            for (i, name) in names.iter().enumerate() {
                                if let Some(name) = name {
                                    let item = items.borrow().get(i).cloned();
                                    scope.define(*name, item.unwrap_or(Value::Undefined));
                                }
                            }
                        }
                    }
                }
            }
            Stmt::Return(value) => {
                let value = match value {
                    Some(value) => self.eval(value, env)?,
                    None => Value::Undefined,
                };
                return Ok(Flow::Return(value));
            }
            Stmt::If(test, then, other) => {
                let test = self.eval(test, env)?;
                if self.truthy(test)? {
                    return self.exec(then, env);
                } else if let Some(other) = other {
                    return self.exec(other, env);
                }
            }
            Stmt::Block(body) => return self.scoped(body, env),
            Stmt::While(test, body) => loop {
                self.step()?;
                let test = self.eval(test, env)?;
                if !self.truthy(test)? {
                    break;
                }
                match self.exec(body, env)? {
                    Flow::Break => break,
                    Flow::Return(value) => return Ok(Flow::Return(value)),
                    Flow::Next | Flow::Continue => {}
                }
            },
            Stmt::DoWhile(body, test) => loop {
                self.step()?;
                match self.exec(body, env)? {
                    Flow::Break => break,
                    Flow::Return(value) => return Ok(Flow::Return(value)),
                    Flow::Next | Flow::Continue => {}
                }
                let test = self.eval(test, env)?;
                if !self.truthy(test)? {
                    break;
                }
            },
            Stmt::For {
                init,
                test,
                update,
                body,
            } => {
                let scope = Env::new(Some(env.clone()), false);
                if let Some(init) = init {
                    self.exec(init, &scope)?;
                }
                loop {
                    self.step()?;
                    if let Some(test) = test {
                        let test = self.eval(test, &scope)?;
                        if !self.truthy(test)? {
                            break;
                        }
                    }
                    match self.exec(body, &scope)? {
                        Flow::Break => break,
                        Flow::Return(value) => return Ok(Flow::Return(value)),
                        Flow::Next | Flow::Continue => {}
                    }
                    if let Some(update) = update {
                        self.eval(update, &scope)?;
                    }
                }
            }
            Stmt::ForEach {
                name,
                keys,
                subject,
                body,
            } => {
                let subject = self.eval(subject, env)?;
                let items: Vec<Value> = match self.plain(subject)? {
                    Value::Array(items) if *keys => (0..items.borrow().len())
                        .map(|i| Value::Num(i as f64))
                        .collect(),
                    Value::Array(items) => items.borrow().clone(),
                    Value::Object(fields) if *keys => fields
                        .borrow()
                        .iter()
                        .map(|(k, _)| Value::Str(k.clone()))
                        .collect(),
                    Value::Str(text) if !*keys => text
                        .chars()
                        .map(|c| Value::Str(Rc::from(c.to_string().as_str())))
                        .collect(),
                    _ => return fail("this cannot be looped over"),
                };
                let scope = Env::new(Some(env.clone()), false);
                for item in items {
                    self.step()?;
                    scope.define(*name, item);
                    match self.exec(body, &scope)? {
                        Flow::Break => break,
                        Flow::Return(value) => return Ok(Flow::Return(value)),
                        Flow::Next | Flow::Continue => {}
                    }
                }
            }
            Stmt::Break => return Ok(Flow::Break),
            Stmt::Continue => return Ok(Flow::Continue),
            Stmt::Throw(value) => {
                let value = self.eval(value, env)?;
                return Err(Abort::Throw(value));
            }
            Stmt::Try {
                body,
                param,
                handler,
                finally,
            } => {
                let mut done = self.scoped(body, env);
                if let (Err(abort), Some(handler)) = (&done, handler) {
                    // Running out of steps is not something to catch: a loop
                    // that never ends inside a `try` would carry on for ever.
                    if !spent_out() {
                        let caught = match abort {
                            Abort::Throw(value) => value.clone(),
                            Abort::Error(why) => {
                                let message = Value::Str(Rc::from(why.as_str()));
                                self.object(vec![(Rc::from("message"), message)])
                            }
                        };
                        let scope = Env::new(Some(env.clone()), false);
                        if let Some(param) = param {
                            scope.define(*param, caught);
                        }
                        done = self.block(handler, &scope);
                    }
                }
                if let Some(finally) = finally {
                    match self.scoped(finally, env)? {
                        Flow::Next => {}
                        other => return Ok(other),
                    }
                }
                return done;
            }
            Stmt::Switch { subject, cases } => {
                let subject = self.eval(subject, env)?;
                let subject = self.plain(subject)?;
                let mut start = None;
                for (i, (test, _)) in cases.iter().enumerate() {
                    if let Some(test) = test {
                        let test = self.eval(test, env)?;
                        let test = self.plain(test)?;
                        if strict_equal(&subject, &test) {
                            start = Some(i);
                            break;
                        }
                    }
                }
                let start = start.or_else(|| cases.iter().position(|(test, _)| test.is_none()));
                if let Some(start) = start {
                    let scope = Env::new(Some(env.clone()), false);
                    for (_, body) in cases.iter().skip(start) {
                        match self.block(body, &scope)? {
                            Flow::Next => {}
                            Flow::Break => break,
                            other => return Ok(other),
                        }
                    }
                }
            }
        }
        Ok(Flow::Next)
    }

    // ───────────────────────────── expressions ────────────────────────────

    fn eval(&mut self, expr: &Expr, env: &Rc<Env>) -> Res<Value> {
        self.step()?;
        Ok(match expr {
            Expr::Num(n) => Value::Num(*n),
            Expr::Str(s) => Value::Str(s.clone()),
            Expr::Bool(b) => Value::Bool(*b),
            Expr::Null => Value::Null,
            Expr::Ident(sym) => match env.get(*sym) {
                Some(value) => value,
                None => {
                    let program = self.program;
                    let name = program.name(*sym);
                    match self.global(name)? {
                        Some(value) => value,
                        None => return fail(format!("'{name}' is not defined")),
                    }
                }
            },
            Expr::Array(items) => {
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(self.eval(item, env)?);
                }
                self.array(out)?
            }
            Expr::Object(fields) => {
                let mut out = Vec::with_capacity(fields.len());
                for (key, value) in fields {
                    out.push((key.clone(), self.eval(value, env)?));
                }
                self.object(out)
            }
            Expr::Template(pieces) => {
                let mut out = String::new();
                for piece in pieces {
                    match piece {
                        Ok(text) => out.push_str(text),
                        Err(hole) => {
                            let value = self.eval(hole, env)?;
                            let value = self.plain(value)?;
                            out.push_str(&self.text(&value));
                        }
                    }
                }
                self.string(out)?
            }
            Expr::Unary(op, operand) => {
                // `typeof` of a name nobody defined is an answer, not an error.
                if let (Unary::TypeOf, Expr::Ident(sym)) = (op, &**operand) {
                    let program = self.program;
                    if env.get(*sym).is_none() && self.global(program.name(*sym))?.is_none() {
                        return Ok(Value::Str(Rc::from("undefined")));
                    }
                }
                let value = self.eval(operand, env)?;
                self.unary(*op, value)?
            }
            Expr::Update {
                target,
                delta,
                prefix,
            } => {
                let old = self.eval(target, env)?;
                let old = self.number(&old)?;
                self.assign(target, Value::Num(old + delta), env)?;
                Value::Num(if *prefix { old + delta } else { old })
            }
            Expr::Binary(op, left, right) => {
                let left = self.eval(left, env)?;
                let right = self.eval(right, env)?;
                self.binary(*op, left, right)?
            }
            Expr::Logical(op, left, right) => {
                let left = self.eval(left, env)?;
                if self.settles(*op, &left)? {
                    left
                } else {
                    self.eval(right, env)?
                }
            }
            Expr::Cond(test, then, other) => {
                let test = self.eval(test, env)?;
                if self.truthy(test)? {
                    self.eval(then, env)?
                } else {
                    self.eval(other, env)?
                }
            }
            Expr::Assign {
                target,
                op,
                logical,
                value,
            } => {
                let value = if let Some(op) = op {
                    let old = self.eval(target, env)?;
                    let value = self.eval(value, env)?;
                    self.binary(*op, old, value)?
                } else if let Some(logical) = logical {
                    let old = self.eval(target, env)?;
                    if self.settles(*logical, &old)? {
                        return Ok(old);
                    }
                    self.eval(value, env)?
                } else {
                    self.eval(value, env)?
                };
                self.assign(target, value.clone(), env)?;
                value
            }
            Expr::Member {
                object,
                name,
                optional,
            } => {
                let object = self.eval(object, env)?;
                if *optional && matches!(object, Value::Undefined | Value::Null) {
                    return Ok(Value::Undefined);
                }
                self.member(object, name)?
            }
            Expr::Index(object, index) => {
                let object = self.eval(object, env)?;
                let index = self.eval(index, env)?;
                self.index(object, index)?
            }
            Expr::Call(callee, args) => {
                // `list.push(x)` is a call *on* the list, so the thing before
                // the dot is kept rather than thrown away after the lookup.
                if let Expr::Member {
                    object,
                    name,
                    optional,
                } = &**callee
                {
                    let object = self.eval(object, env)?;
                    if *optional && matches!(object, Value::Undefined | Value::Null) {
                        return Ok(Value::Undefined);
                    }
                    let args = self.arguments(args, env)?;
                    return self.invoke(object, name, args);
                }
                let callee = self.eval(callee, env)?;
                let args = self.arguments(args, env)?;
                self.call(callee, args)?
            }
            Expr::Function(func) => self.closure(func, env),
            Expr::Seq(all) => {
                let mut last = Value::Undefined;
                for expr in all {
                    last = self.eval(expr, env)?;
                }
                last
            }
        })
    }

    fn arguments(&mut self, args: &[Expr], env: &Rc<Env>) -> Res<Vec<Value>> {
        let mut out = Vec::with_capacity(args.len());
        for arg in args {
            out.push(self.eval(arg, env)?);
        }
        Ok(out)
    }

    /// Whether a logical operator already has its answer in its left side.
    fn settles(&mut self, op: Logical, left: &Value) -> Res<bool> {
        Ok(match op {
            Logical::And => !self.truthy(left.clone())?,
            Logical::Or => self.truthy(left.clone())?,
            Logical::Coalesce => !matches!(left, Value::Undefined | Value::Null),
        })
    }

    fn assign(&mut self, target: &Expr, value: Value, env: &Rc<Env>) -> Res<()> {
        match target {
            Expr::Ident(sym) => {
                if !env.set(*sym, &value) {
                    self.root.define(*sym, value);
                }
            }
            Expr::Member { object, name, .. } => {
                let object = self.eval(object, env)?;
                self.set_field(object, name, value)?;
            }
            Expr::Index(object, index) => {
                let object = self.eval(object, env)?;
                let index = self.eval(index, env)?;
                let index = self.plain(index)?;
                match (&object, &index) {
                    (Value::Array(items), Value::Num(n)) if *n >= 0.0 => {
                        let i = *n as usize;
                        if i >= MAX_ITEMS {
                            return fail("a list too long to hold");
                        }
                        let grown = (i + 1).saturating_sub(items.borrow().len());
                        self.spend(grown)?;
                        let mut items = items.borrow_mut();
                        if i >= items.len() {
                            items.resize(i + 1, Value::Undefined);
                        }
                        if let Some(slot) = items.get_mut(i) {
                            *slot = value;
                        }
                    }
                    _ => {
                        let name = self.text(&index);
                        self.set_field(object, &name, value)?;
                    }
                }
            }
            _ => return fail("this cannot be assigned to"),
        }
        Ok(())
    }

    fn set_field(&mut self, object: Value, name: &str, value: Value) -> Res<()> {
        let Value::Object(fields) = object else {
            return fail(format!("'{name}' cannot be set on this"));
        };
        let mut fields = fields.borrow_mut();
        if let Some((_, slot)) = fields.iter_mut().find(|(k, _)| &**k == name) {
            *slot = value;
        } else if fields.len() >= MAX_ITEMS {
            return fail("an object too large to hold");
        } else {
            fields.push((Rc::from(name), value));
        }
        Ok(())
    }

    pub(super) fn call(&mut self, callee: Value, args: Vec<Value>) -> Res<Value> {
        match callee {
            Value::Func(closure) => {
                if self.calls >= MAX_CALL_DEPTH {
                    return fail("functions call each other too deeply");
                }
                self.calls += 1;
                let scope = Env::new(Some(closure.env.clone()), true);
                let mut args = args.into_iter();
                for param in &closure.func.params {
                    scope.define(*param, args.next().unwrap_or(Value::Undefined));
                }
                // What a function's own statements came to is not what the
                // expression came to.
                let last = std::mem::replace(&mut self.last, Value::Undefined);
                let func = closure.func.clone();
                let done = self.block(&func.body, &scope);
                self.last = last;
                self.calls -= 1;
                Ok(match done? {
                    Flow::Return(value) => value,
                    _ => Value::Undefined,
                })
            }
            Value::Bound(bound) => {
                let (on, name) = &*bound;
                self.method(on.clone(), name, args)
            }
            Value::Host(host) => self.call_host(host, args),
            _ => fail("this is not a function"),
        }
    }

    /// Call `name` on `object`.
    fn invoke(&mut self, object: Value, name: &str, args: Vec<Value>) -> Res<Value> {
        if let Value::Object(fields) = &object {
            let field = fields
                .borrow()
                .iter()
                .find(|(k, _)| &**k == name)
                .map(|(_, v)| v.clone());
            return match field {
                Some(field) => self.call(field, args),
                None => fail(format!("'{name}' is not a function")),
            };
        }
        self.method(object, name, args)
    }

    fn member(&mut self, object: Value, name: &Rc<str>) -> Res<Value> {
        Ok(match &object {
            Value::Undefined | Value::Null => {
                return fail(format!("'{name}' was read on something that is not there"))
            }
            Value::Array(items) if &**name == "length" => Value::Num(items.borrow().len() as f64),
            Value::Str(text) if &**name == "length" => {
                self.spend(text.len() / 64)?;
                Value::Num(text.chars().count() as f64)
            }
            Value::Object(fields) => fields
                .borrow()
                .iter()
                .find(|(k, _)| k == name)
                .map_or(Value::Undefined, |(_, v)| v.clone()),
            Value::Host(host) => match self.host_member(host, name)? {
                Some(value) => value,
                None => Value::Bound(Rc::new((object.clone(), name.clone()))),
            },
            Value::Func(_) | Value::Bound(_) => Value::Undefined,
            _ => Value::Bound(Rc::new((object.clone(), name.clone()))),
        })
    }

    fn index(&mut self, object: Value, index: Value) -> Res<Value> {
        let object = self.plain(object)?;
        let index = self.plain(index)?;
        Ok(match (&object, &index) {
            (Value::Array(items), Value::Num(n)) => {
                let items = items.borrow();
                if *n >= 0.0 && n.fract() == 0.0 {
                    items.get(*n as usize).cloned().unwrap_or(Value::Undefined)
                } else {
                    Value::Undefined
                }
            }
            (Value::Str(text), Value::Num(n)) if *n >= 0.0 => {
                self.spend(text.len() / 64)?;
                text.chars().nth(*n as usize).map_or(Value::Undefined, |c| {
                    Value::Str(Rc::from(c.to_string().as_str()))
                })
            }
            (Value::Undefined | Value::Null, _) => {
                return fail("an item was read on something that is not there")
            }
            _ => {
                let name: Rc<str> = Rc::from(self.text(&index).as_str());
                self.member(object, &name)?
            }
        })
    }

    // ───────────────────────────── conversions ────────────────────────────

    /// A property stands for its value wherever a value is wanted, which is
    /// what lets `effect("Shake")("Seed") * 2` read as it does.
    pub(super) fn plain(&mut self, value: Value) -> Res<Value> {
        match value {
            Value::Host(Host::Prop(prop)) => self.prop_value(&prop),
            other => Ok(other),
        }
    }

    pub(super) fn number(&mut self, value: &Value) -> Res<f64> {
        Ok(match value {
            Value::Host(Host::Prop(prop)) => {
                let value = self.prop_value(prop)?;
                to_number(&value)
            }
            other => to_number(other),
        })
    }

    pub(super) fn truthy(&mut self, value: Value) -> Res<bool> {
        Ok(match self.plain(value)? {
            Value::Undefined | Value::Null => false,
            Value::Bool(b) => b,
            Value::Num(n) => n != 0.0 && !n.is_nan(),
            Value::Str(s) => !s.is_empty(),
            _ => true,
        })
    }

    /// A value as the words it prints as. Properties have been made plain by
    /// the caller.
    pub(super) fn text(&self, value: &Value) -> String {
        let mut out = String::new();
        write_text(value, 0, &mut out);
        out
    }

    /// The numbers in a value: one for a number, each item for a list.
    pub(super) fn vector(&mut self, value: &Value) -> Res<Vec<f64>> {
        Ok(match self.plain(value.clone())? {
            Value::Array(items) => {
                let items = items.borrow().clone();
                let mut out = Vec::with_capacity(items.len());
                for item in &items {
                    out.push(self.number(item)?);
                }
                out
            }
            other => vec![to_number(&other)],
        })
    }

    /// A list of numbers as a value again: one number stays a number.
    pub(super) fn value_of(&mut self, numbers: &[f64], as_list: bool) -> Res<Value> {
        match numbers {
            [one] if !as_list => Ok(Value::Num(*one)),
            many => self.numbers(many),
        }
    }

    // ───────────────────────────── operators ──────────────────────────────

    fn unary(&mut self, op: Unary, value: Value) -> Res<Value> {
        let value = self.plain(value)?;
        Ok(match op {
            Unary::Not => Value::Bool(!self.truthy(value)?),
            Unary::Neg => match &value {
                Value::Array(_) => {
                    let numbers: Vec<f64> = self.vector(&value)?.iter().map(|n| -n).collect();
                    self.numbers(&numbers)?
                }
                other => Value::Num(-to_number(other)),
            },
            Unary::Plus => Value::Num(to_number(&value)),
            Unary::BitNot => Value::Num(f64::from(!to_int(to_number(&value)))),
            Unary::Void => Value::Undefined,
            Unary::TypeOf => Value::Str(Rc::from(match value {
                Value::Undefined => "undefined",
                Value::Bool(_) => "boolean",
                Value::Num(_) => "number",
                Value::Str(_) => "string",
                Value::Func(_) | Value::Bound(_) => "function",
                _ => "object",
            })),
        })
    }

    /// Arithmetic between two values, either of which may be a list.
    ///
    /// After Effects reads `[10, 20] + [1, 2]` as `[11, 22]` and
    /// `[10, 20] * 2` as `[20, 40]`, where JavaScript proper would glue two
    /// pieces of text together. Nearly every expression on a position leans
    /// on this, so it is what the operators do here.
    fn spread(
        &mut self,
        left: &Value,
        right: &Value,
        op: impl Fn(f64, f64) -> f64,
        pad: bool,
    ) -> Res<Value> {
        let lists = matches!(left, Value::Array(_)) || matches!(right, Value::Array(_));
        if !lists {
            return Ok(Value::Num(op(to_number(left), to_number(right))));
        }
        let a = self.vector(left)?;
        let b = self.vector(right)?;
        let scale_a = !matches!(left, Value::Array(_)) && !pad;
        let scale_b = !matches!(right, Value::Array(_)) && !pad;
        let len = a.len().max(b.len());
        let at = |list: &[f64], i: usize, scale: bool| {
            if scale {
                list.first().copied().unwrap_or(0.0)
            } else {
                list.get(i).copied().unwrap_or(0.0)
            }
        };
        let out: Vec<f64> = (0..len)
            .map(|i| op(at(&a, i, scale_a), at(&b, i, scale_b)))
            .collect();
        self.numbers(&out)
    }

    pub(super) fn binary(&mut self, op: Binary, left: Value, right: Value) -> Res<Value> {
        let left = self.plain(left)?;
        let right = self.plain(right)?;
        let (a, b) = (&left, &right);
        Ok(match op {
            Binary::Add => {
                if matches!(a, Value::Str(_)) || matches!(b, Value::Str(_)) {
                    self.string(format!("{}{}", self.text(a), self.text(b)))?
                } else {
                    // A number added to a list joins its first item.
                    self.spread(a, b, |x, y| x + y, true)?
                }
            }
            Binary::Sub => self.spread(a, b, |x, y| x - y, true)?,
            // A number times a list scales every item.
            Binary::Mul => self.spread(a, b, |x, y| x * y, false)?,
            Binary::Div => self.spread(a, b, |x, y| x / y, false)?,
            Binary::Rem => Value::Num(to_number(a) % to_number(b)),
            Binary::Pow => Value::Num(to_number(a).powf(to_number(b))),
            Binary::Eq => Value::Bool(loose_equal(a, b)),
            Binary::Ne => Value::Bool(!loose_equal(a, b)),
            Binary::StrictEq => Value::Bool(strict_equal(a, b)),
            Binary::StrictNe => Value::Bool(!strict_equal(a, b)),
            Binary::Lt | Binary::Le | Binary::Gt | Binary::Ge => {
                let order = match (a, b) {
                    (Value::Str(x), Value::Str(y)) => Some(x.cmp(y)),
                    _ => to_number(a).partial_cmp(&to_number(b)),
                };
                Value::Bool(order.is_some_and(|order| match op {
                    Binary::Lt => order.is_lt(),
                    Binary::Le => order.is_le(),
                    Binary::Gt => order.is_gt(),
                    _ => order.is_ge(),
                }))
            }
            Binary::BitAnd => Value::Num(f64::from(to_int(to_number(a)) & to_int(to_number(b)))),
            Binary::BitOr => Value::Num(f64::from(to_int(to_number(a)) | to_int(to_number(b)))),
            Binary::BitXor => Value::Num(f64::from(to_int(to_number(a)) ^ to_int(to_number(b)))),
            Binary::Shl => Value::Num(f64::from(
                to_int(to_number(a)).wrapping_shl(to_int(to_number(b)) as u32 & 31),
            )),
            Binary::Shr => Value::Num(f64::from(
                to_int(to_number(a)).wrapping_shr(to_int(to_number(b)) as u32 & 31),
            )),
            Binary::UShr => Value::Num(f64::from(
                (to_int(to_number(a)) as u32).wrapping_shr(to_int(to_number(b)) as u32 & 31),
            )),
        })
    }

    // ───────────────────────────── the library ────────────────────────────

    /// Call the built-in `name` on `on`.
    pub(super) fn method(&mut self, on: Value, name: &str, args: Vec<Value>) -> Res<Value> {
        match on {
            Value::Host(host) => self.host_method(host, name, args),
            Value::Array(items) => self.array_method(&items, name, args),
            Value::Str(text) => self.text_method(&text, name, args),
            Value::Num(n) => match name {
                "toFixed" => {
                    let digits = match args.first() {
                        Some(digits) => self.number(digits)?.clamp(0.0, 20.0) as usize,
                        None => 0,
                    };
                    self.string(format!("{n:.digits$}"))
                }
                "toString" => self.string(number_text(n)),
                _ => fail(format!("a number has no '{name}'")),
            },
            _ => fail(format!("'{name}' is not a function")),
        }
    }

    pub(super) fn arg(&mut self, args: &[Value], i: usize) -> Res<f64> {
        match args.get(i) {
            Some(value) => self.number(value),
            None => Ok(f64::NAN),
        }
    }

    pub(super) fn math(&mut self, name: &str, args: &[Value]) -> Res<Value> {
        let x = self.arg(args, 0)?;
        Ok(Value::Num(match name {
            "abs" => x.abs(),
            "acos" => x.acos(),
            "asin" => x.asin(),
            "atan" => x.atan(),
            "atan2" => x.atan2(self.arg(args, 1)?),
            "cbrt" => x.cbrt(),
            "ceil" => x.ceil(),
            "cos" => x.cos(),
            "exp" => x.exp(),
            "floor" => x.floor(),
            "log" => x.ln(),
            "log2" => x.log2(),
            "log10" => x.log10(),
            "pow" => x.powf(self.arg(args, 1)?),
            // JavaScript rounds a half upwards, so -2.5 becomes -2.
            "round" => (x + 0.5).floor(),
            "sign" => {
                if x > 0.0 {
                    1.0
                } else if x < 0.0 {
                    -1.0
                } else {
                    x
                }
            }
            "sin" => x.sin(),
            "sqrt" => x.sqrt(),
            "tan" => x.tan(),
            "trunc" => x.trunc(),
            "random" => self.ae.random(),
            "hypot" | "max" | "min" => {
                let mut numbers = Vec::with_capacity(args.len());
                for arg in args {
                    numbers.push(self.number(arg)?);
                }
                match name {
                    "hypot" => numbers.iter().map(|n| n * n).sum::<f64>().sqrt(),
                    _ if numbers.iter().any(|n| n.is_nan()) => f64::NAN,
                    "max" => numbers.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                    _ => numbers.iter().copied().fold(f64::INFINITY, f64::min),
                }
            }
            _ => return fail(format!("Math has no '{name}'")),
        }))
    }

    fn array_method(
        &mut self,
        items: &Rc<RefCell<Vec<Value>>>,
        name: &str,
        args: Vec<Value>,
    ) -> Res<Value> {
        // A copy to walk, so a callback that changes the list it is being
        // called over cannot pull it out from under the loop.
        let list = || items.borrow().clone();
        // Everything but the two that only touch the end walks the list.
        if !matches!(name, "push" | "pop") {
            self.spend(items.borrow().len() / 8)?;
        }
        let first = args.first().cloned().unwrap_or(Value::Undefined);
        Ok(match name {
            "push" => {
                let mut items = items.borrow_mut();
                if items.len() + args.len() > MAX_ITEMS {
                    return fail("a list too long to hold");
                }
                items.extend(args);
                Value::Num(items.len() as f64)
            }
            "pop" => items.borrow_mut().pop().unwrap_or(Value::Undefined),
            "shift" => {
                let mut items = items.borrow_mut();
                if items.is_empty() {
                    Value::Undefined
                } else {
                    items.remove(0)
                }
            }
            "unshift" => {
                let mut items = items.borrow_mut();
                if items.len() + args.len() > MAX_ITEMS {
                    return fail("a list too long to hold");
                }
                items.splice(0..0, args);
                Value::Num(items.len() as f64)
            }
            "slice" => {
                let list = list();
                let (from, to) = self.range(&args, list.len())?;
                self.array(list.get(from..to).unwrap_or_default().to_vec())?
            }
            "concat" => {
                let mut out = list();
                for arg in args {
                    match self.plain(arg)? {
                        Value::Array(more) => out.extend(more.borrow().iter().cloned()),
                        other => out.push(other),
                    }
                }
                self.array(out)?
            }
            "indexOf" | "includes" => {
                let first = self.plain(first)?;
                let at = list().iter().position(|item| strict_equal(item, &first));
                if name == "includes" {
                    Value::Bool(at.is_some())
                } else {
                    Value::Num(at.map_or(-1.0, |i| i as f64))
                }
            }
            "join" => {
                let glue = match &first {
                    Value::Undefined => ",".to_string(),
                    other => self.text(other),
                };
                let mut parts = Vec::new();
                for item in list() {
                    let item = self.plain(item)?;
                    parts.push(match item {
                        Value::Undefined | Value::Null => String::new(),
                        other => self.text(&other),
                    });
                }
                self.string(parts.join(&glue))?
            }
            "reverse" => {
                items.borrow_mut().reverse();
                Value::Array(items.clone())
            }
            "map" | "filter" | "forEach" | "some" | "every" | "find" | "findIndex" => {
                let mut mapped = Vec::new();
                for (i, item) in list().into_iter().enumerate() {
                    self.step()?;
                    let answer = self.call(
                        first.clone(),
                        vec![
                            item.clone(),
                            Value::Num(i as f64),
                            Value::Array(items.clone()),
                        ],
                    )?;
                    match name {
                        "map" => mapped.push(answer),
                        "forEach" => {}
                        _ => {
                            let hit = self.truthy(answer)?;
                            match name {
                                "filter" if hit => mapped.push(item),
                                "some" if hit => return Ok(Value::Bool(true)),
                                "every" if !hit => return Ok(Value::Bool(false)),
                                "find" if hit => return Ok(item),
                                "findIndex" if hit => return Ok(Value::Num(i as f64)),
                                _ => {}
                            }
                        }
                    }
                }
                match name {
                    "map" | "filter" => self.array(mapped)?,
                    "some" => Value::Bool(false),
                    "every" => Value::Bool(true),
                    "findIndex" => Value::Num(-1.0),
                    _ => Value::Undefined,
                }
            }
            "reduce" => {
                let mut list = list().into_iter().enumerate();
                let mut total = match args.get(1) {
                    Some(start) => start.clone(),
                    None => match list.next() {
                        Some((_, item)) => item,
                        None => return fail("an empty list reduces to nothing"),
                    },
                };
                for (i, item) in list {
                    self.step()?;
                    total = self.call(first.clone(), vec![total, item, Value::Num(i as f64)])?;
                }
                total
            }
            "sort" => {
                let mut list = list();
                // An insertion sort: the comparison is a call back into the
                // expression, which can fail, and a short list is the only
                // kind an expression sorts.
                for i in 1..list.len() {
                    let mut j = i;
                    while j > 0 {
                        self.step()?;
                        let (Some(a), Some(b)) = (list.get(j - 1).cloned(), list.get(j).cloned())
                        else {
                            break;
                        };
                        let after = match &first {
                            Value::Undefined => self.text(&a) > self.text(&b),
                            compare => {
                                let order = self.call(compare.clone(), vec![a, b])?;
                                self.number(&order)? > 0.0
                            }
                        };
                        if !after {
                            break;
                        }
                        list.swap(j - 1, j);
                        j -= 1;
                    }
                }
                *items.borrow_mut() = list;
                Value::Array(items.clone())
            }
            "toString" => {
                let items = Value::Array(items.clone());
                self.string(self.text(&items))?
            }
            _ => return fail(format!("a list has no '{name}'")),
        })
    }

    /// The `(start, end)` a `slice` asks for, counted from the end when
    /// negative, inside a thing `len` long.
    fn range(&mut self, args: &[Value], len: usize) -> Res<(usize, usize)> {
        let mut at = |i: usize, missing: f64| -> Res<usize> {
            let n = match args.get(i) {
                None | Some(Value::Undefined) => missing,
                Some(value) => self.number(value)?,
            };
            let n = if n < 0.0 { n + len as f64 } else { n };
            Ok(n.clamp(0.0, len as f64) as usize)
        };
        let from = at(0, 0.0)?;
        let to = at(1, len as f64)?;
        Ok((from, to.max(from)))
    }

    fn text_method(&mut self, text: &Rc<str>, name: &str, args: Vec<Value>) -> Res<Value> {
        let first = match args.first() {
            Some(first) => {
                let first = self.plain(first.clone())?;
                Some(self.text(&first))
            }
            None => None,
        };
        let needle = first.clone().unwrap_or_default();
        self.spend(text.len() / 64)?;
        let chars: Vec<char> = text.chars().collect();
        let char_index = |byte: Option<usize>| {
            byte.map_or(-1.0, |byte| {
                text.get(..byte).map_or(0, |s| s.chars().count()) as f64
            })
        };
        Ok(match name {
            "toUpperCase" => self.string(text.to_uppercase())?,
            "toLowerCase" => self.string(text.to_lowercase())?,
            "trim" => self.string(text.trim().to_string())?,
            "toString" => Value::Str(text.clone()),
            "indexOf" => Value::Num(char_index(text.find(&needle))),
            "lastIndexOf" => Value::Num(char_index(text.rfind(&needle))),
            "includes" => Value::Bool(text.contains(&needle)),
            "startsWith" => Value::Bool(text.starts_with(&needle)),
            "endsWith" => Value::Bool(text.ends_with(&needle)),
            "charAt" => {
                let i = self.arg(&args, 0)?;
                let c = if i >= 0.0 {
                    chars.get(i as usize)
                } else {
                    None
                };
                self.string(c.map(char::to_string).unwrap_or_default())?
            }
            "charCodeAt" => {
                let i = self.arg(&args, 0)?;
                let c = if i >= 0.0 {
                    chars.get(i as usize)
                } else {
                    None
                };
                Value::Num(c.map_or(f64::NAN, |c| f64::from(u32::from(*c))))
            }
            "slice" | "substring" => {
                let (from, to) = self.range(&args, chars.len())?;
                self.string(chars.get(from..to).unwrap_or_default().iter().collect())?
            }
            "substr" => {
                let (from, _) = self.range(&args, chars.len())?;
                let count = match args.get(1) {
                    Some(count) => self.number(count)?.max(0.0) as usize,
                    None => chars.len(),
                };
                self.string(chars.iter().skip(from).take(count).collect())?
            }
            "split" => {
                let parts: Vec<Value> = match &first {
                    None => vec![Value::Str(text.clone())],
                    Some(glue) if glue.is_empty() => chars
                        .iter()
                        .map(|c| Value::Str(Rc::from(c.to_string().as_str())))
                        .collect(),
                    Some(glue) => text
                        .split(glue.as_str())
                        .map(|part| Value::Str(Rc::from(part)))
                        .collect(),
                };
                self.array(parts)?
            }
            "replace" => {
                let with = match args.get(1) {
                    Some(with) => {
                        let with = self.plain(with.clone())?;
                        self.text(&with)
                    }
                    None => "undefined".into(),
                };
                self.string(text.replacen(&needle, &with, 1))?
            }
            "repeat" => {
                let count = self.arg(&args, 0)?.max(0.0) as usize;
                if text.len().saturating_mul(count) > MAX_TEXT_BYTES {
                    return fail("text too long to hold");
                }
                self.string(text.repeat(count))?
            }
            "padStart" | "padEnd" => {
                let width = self.arg(&args, 0)?.clamp(0.0, 4096.0) as usize;
                let fill = match args.get(1) {
                    Some(fill) => {
                        let fill = self.plain(fill.clone())?;
                        self.text(&fill)
                    }
                    None => " ".into(),
                };
                let missing = width.saturating_sub(chars.len());
                let pad: String = fill.chars().cycle().take(missing).collect();
                if fill.is_empty() {
                    Value::Str(text.clone())
                } else if name == "padStart" {
                    self.string(format!("{pad}{text}"))?
                } else {
                    self.string(format!("{text}{pad}"))?
                }
            }
            _ => return fail(format!("text has no '{name}'")),
        })
    }
}

/// A value as a number, the way JavaScript's arithmetic reads it. Properties
/// have been made plain by the caller.
pub(super) fn to_number(value: &Value) -> f64 {
    number_at(value, 0)
}

fn number_at(value: &Value, depth: u32) -> f64 {
    match value {
        Value::Num(n) => *n,
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Null => 0.0,
        Value::Str(s) => {
            let s = s.trim();
            if s.is_empty() {
                0.0
            } else {
                s.parse().unwrap_or(f64::NAN)
            }
        }
        Value::Array(items) if depth < MAX_LIST_DEPTH => match items.borrow().as_slice() {
            [] => 0.0,
            [one] => number_at(one, depth + 1),
            _ => f64::NAN,
        },
        _ => f64::NAN,
    }
}

/// [`Interp::text`], written into `out`. It stops once `out` is longer than
/// any text may be, so a list of lists of lists is not built in full first,
/// and a list past [`MAX_LIST_DEPTH`] reads as nothing, as one that holds
/// itself does in JavaScript.
fn write_text(value: &Value, depth: u32, out: &mut String) {
    if out.len() > MAX_TEXT_BYTES {
        return;
    }
    match value {
        Value::Undefined => out.push_str("undefined"),
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Num(n) => out.push_str(&number_text(*n)),
        Value::Str(s) => out.push_str(s),
        Value::Array(items) if depth < MAX_LIST_DEPTH => {
            for (i, item) in items.borrow().iter().enumerate() {
                if out.len() > MAX_TEXT_BYTES {
                    return;
                }
                if i > 0 {
                    out.push(',');
                }
                if !matches!(item, Value::Undefined | Value::Null) {
                    write_text(item, depth + 1, out);
                }
            }
        }
        Value::Array(_) => {}
        Value::Object(_) => out.push_str("[object Object]"),
        Value::Func(_) | Value::Bound(_) => out.push_str("function"),
        Value::Host(_) => out.push_str("[object]"),
    }
}

/// A number as the 32-bit whole number the bitwise operators work on.
fn to_int(n: f64) -> i32 {
    if n.is_finite() {
        // Wrapping, as JavaScript does, rather than Rust's saturating cast.
        (n.trunc() as i64) as i32
    } else {
        0
    }
}

/// A number as JavaScript prints it: a whole number has no `.0`.
pub(super) fn number_text(n: f64) -> String {
    if n.is_nan() {
        "NaN".into()
    } else if n.is_infinite() {
        if n > 0.0 { "Infinity" } else { "-Infinity" }.into()
    } else if n == n.trunc() && n.abs() < 1e21 {
        format!("{n:.0}")
    } else {
        format!("{n}")
    }
}

pub(super) fn strict_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Undefined, Value::Undefined) | (Value::Null, Value::Null) => true,
        (Value::Bool(x), Value::Bool(y)) => x == y,
        (Value::Num(x), Value::Num(y)) => x == y,
        (Value::Str(x), Value::Str(y)) => x == y,
        (Value::Array(x), Value::Array(y)) => Rc::ptr_eq(x, y),
        (Value::Object(x), Value::Object(y)) => Rc::ptr_eq(x, y),
        (Value::Func(x), Value::Func(y)) => Rc::ptr_eq(x, y),
        _ => false,
    }
}

fn loose_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Undefined | Value::Null, Value::Undefined | Value::Null) => true,
        (Value::Undefined | Value::Null, _) | (_, Value::Undefined | Value::Null) => false,
        (Value::Num(_) | Value::Bool(_), Value::Num(_) | Value::Bool(_) | Value::Str(_))
        | (Value::Str(_), Value::Num(_) | Value::Bool(_)) => to_number(a) == to_number(b),
        _ => strict_equal(a, b),
    }
}
