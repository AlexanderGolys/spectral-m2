//! Persistent completion frequencies and references to symbols outside local scopes.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, Write};
use std::path::Path;

use m2_syn::nodes::{
    AssignOperator, Expr, ExprAssign, ExprField, ExprFor, ExprFunction, ExprQuote, QuoteKeyword,
    Symbol, TryFallback,
};
use m2_syn::visit::{self, Visit};
use serde::{Deserialize, Serialize};

use crate::ui::settings::{config_file, Persisted};

/// Counts of global symbol references from evaluated source.
#[derive(Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Usage(BTreeMap<String, u64>);

impl Persisted for Usage {
    const FILE: &'static str = "completion-usage.json";
}

impl Usage {
    /// Read the frequencies saved by earlier sessions.
    pub fn load() -> Self {
        config_file(Self::FILE)
            .and_then(|path| read_file(&path).ok())
            .unwrap_or_default()
    }

    /// References counted for one name.
    pub fn count(&self, name: &str) -> u64 {
        self.0.get(name).copied().unwrap_or_default()
    }

    /// Merge references into the most recently saved counts and retain them.
    pub fn record(&mut self, names: Vec<String>) {
        if names.is_empty() {
            return;
        }
        if let Some(saved) =
            config_file(Self::FILE).and_then(|path| record_file(&path, &names).ok())
        {
            *self = saved;
        } else {
            self.count_references(&names);
        }
    }

    fn count_references(&mut self, names: &[String]) {
        for name in names {
            let count = self.0.entry(name.clone()).or_default();
            *count = count.saturating_add(1);
        }
    }
}

/// Read a complete snapshot while another shell may be updating it.
fn read_file(path: &Path) -> io::Result<Usage> {
    let file = File::open(path)?;
    file.lock_shared()?;
    Ok(serde_json::from_reader(file).unwrap_or_default())
}

/// Merge one cell's references under a file lock, so concurrent shells keep every use.
fn record_file(path: &Path, names: &[String]) -> io::Result<Usage> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.lock()?;
    let mut usage: Usage = serde_json::from_reader(&file).unwrap_or_default();
    usage.count_references(names);
    let bytes = serde_json::to_vec(&usage)?;
    file.rewind()?;
    file.write_all(&bytes)?;
    file.set_len(bytes.len() as u64)?;
    file.flush()?;
    Ok(usage)
}

/// Global references in source; retain top-level locals for the next cell.
pub fn references(source: &str, locals: &mut BTreeSet<String>) -> Vec<String> {
    let Ok(file) = m2_syn::parse_native(source) else {
        return Vec::new();
    };
    let mut references = References {
        scopes: vec![std::mem::take(locals)],
        names: Vec::new(),
    };
    references.visit_source_file(&file);
    *locals = references.scopes.pop().unwrap_or_default();
    references.names
}

/// Names collected from a binding pattern, without counting them as uses.
#[derive(Default)]
struct Bindings(Vec<String>);

impl<'ast> Visit<'ast> for Bindings {
    fn visit_symbol(&mut self, symbol: &'ast Symbol) {
        self.0.push(symbol.text().to_string());
    }
}

/// Lexical local scopes surrounding the references being counted.
struct References {
    scopes: Vec<BTreeSet<String>>,
    names: Vec<String>,
}

impl References {
    fn bind(&mut self, name: &Symbol) {
        self.scopes
            .last_mut()
            .expect("a top-level scope")
            .insert(name.text().to_string());
    }

    fn scoped(&mut self, names: Vec<String>, walk: impl FnOnce(&mut Self)) {
        self.scopes.push(names.into_iter().collect());
        walk(self);
        self.scopes.pop();
    }
}

impl<'ast> Visit<'ast> for References {
    fn visit_symbol(&mut self, symbol: &'ast Symbol) {
        if !self
            .scopes
            .iter()
            .rev()
            .any(|scope| scope.contains(symbol.text()))
        {
            self.names.push(symbol.text().to_string());
        }
    }

    fn visit_expr_assign(&mut self, assignment: &'ast ExprAssign) {
        match (&assignment.op, assignment.left.as_ref()) {
            (AssignOperator::Local(_), Expr::Symbol(symbol)) => self.bind(symbol),
            (AssignOperator::Local(_), Expr::Sequence(sequence)) => {
                let mut bindings = Bindings::default();
                bindings.visit_expr_sequence(sequence);
                self.scopes
                    .last_mut()
                    .expect("a top-level scope")
                    .extend(bindings.0);
            }
            (AssignOperator::Global(_), Expr::Symbol(_) | Expr::Sequence(_)) => {}
            _ => self.visit_expr(&assignment.left),
        }
        self.visit_expr(&assignment.right);
    }

    fn visit_expr_function(&mut self, lambda: &'ast ExprFunction) {
        let mut bindings = Bindings::default();
        bindings.visit_function_params(&lambda.params);
        self.scoped(bindings.0, |this| this.visit_expr(&lambda.body));
    }

    fn visit_expr_for(&mut self, expression: &'ast ExprFor) {
        if let Some(iter) = &expression.iter {
            self.visit_for_iter(iter);
        }
        self.scoped(vec![expression.var.text().to_string()], |this| {
            if let Some(condition) = &expression.when_clause {
                this.visit_when_clause(condition);
            }
            this.visit_loop_body(&expression.body);
        });
    }

    fn visit_try_fallback(&mut self, fallback: &'ast TryFallback) {
        match fallback {
            TryFallback::Except(value) => self.scoped(vec![value.var.text().to_string()], |this| {
                this.visit_expr(&value.expr)
            }),
            _ => visit::visit_try_fallback(self, fallback),
        }
    }

    fn visit_expr_quote(&mut self, quote: &'ast ExprQuote) {
        match &quote.keyword {
            QuoteKeyword::Local(_) | QuoteKeyword::ThreadLocal(_) => self.bind(&quote.symbol),
            QuoteKeyword::Global(_) => self.names.push(quote.symbol.text().to_string()),
            _ => visit::visit_expr_quote(self, quote),
        }
    }

    fn visit_expr_field(&mut self, member: &'ast ExprField) {
        self.visit_expr(&member.base);
    }
}
