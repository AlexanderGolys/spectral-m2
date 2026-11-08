//! Persistent completion frequencies and references to symbols outside local scopes.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, Write};
use std::path::Path;

use m2_syn::nodes::{
    ExprAssignParallelGlobal, ExprAssignParallelLocal, ExprAssignSimpleGlobal,
    ExprAssignSimpleLocal, ExprFor, ExprLambdaList, ExprLambdaVariadic, ExprOpBinMember, ExprQuote,
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

    fn visit_expr_assign_simple_local(&mut self, assignment: &'ast ExprAssignSimpleLocal) {
        self.bind(&assignment.variable);
        self.visit_expr(&assignment.value);
    }

    fn visit_expr_assign_simple_global(&mut self, assignment: &'ast ExprAssignSimpleGlobal) {
        self.visit_expr(&assignment.value);
    }

    fn visit_expr_assign_parallel_local(&mut self, assignment: &'ast ExprAssignParallelLocal) {
        let mut bindings = Bindings::default();
        bindings.visit_assignment_list(&assignment.argument_list);
        self.scopes
            .last_mut()
            .expect("a top-level scope")
            .extend(bindings.0);
        self.visit_expr(&assignment.value);
    }

    fn visit_expr_assign_parallel_global(&mut self, assignment: &'ast ExprAssignParallelGlobal) {
        self.visit_expr(&assignment.value);
    }

    fn visit_expr_lambda_variadic(&mut self, lambda: &'ast ExprLambdaVariadic) {
        self.scoped(vec![lambda.argument.text().to_string()], |this| {
            this.visit_expr(&lambda.body)
        });
    }

    fn visit_expr_lambda_list(&mut self, lambda: &'ast ExprLambdaList) {
        let mut bindings = Bindings::default();
        if let Some(arguments) = &lambda.arguments.contents {
            bindings.visit_argument_list_content(arguments);
        }
        self.scoped(bindings.0, |this| this.visit_expr(&lambda.body));
    }

    fn visit_expr_for(&mut self, expression: &'ast ExprFor) {
        let (variable, condition, body) = match expression {
            ExprFor::Bare(value) => (&value.variable, &value.condition, &value.body),
            ExprFor::Range(value) => {
                if let Some(bound) = &value.lower_bound {
                    self.visit_range_bound_from(bound);
                }
                if let Some(bound) = &value.upper_bound {
                    self.visit_range_bound_to(bound);
                }
                (&value.variable, &value.condition, &value.body)
            }
            ExprFor::In(value) => {
                self.visit_expr(&value.collection);
                (&value.variable, &value.condition, &value.body)
            }
        };
        self.scoped(vec![variable.text().to_string()], |this| {
            if let Some(condition) = condition {
                this.visit_when_condition(condition);
            }
            this.visit_loop_body(body);
        });
    }

    fn visit_try_fallback(&mut self, fallback: &'ast TryFallback) {
        match fallback {
            TryFallback::Except(value) => self
                .scoped(vec![value.exception.text().to_string()], |this| {
                    this.visit_expr(&value.value)
                }),
            _ => visit::visit_try_fallback(self, fallback),
        }
    }

    fn visit_expr_quote(&mut self, quote: &'ast ExprQuote) {
        match quote {
            ExprQuote::Local(value) => self.bind(&value.symbol),
            ExprQuote::ThreadLocal(value) => self.bind(&value.symbol),
            ExprQuote::Global(value) => self.names.push(value.symbol.text().to_string()),
            _ => visit::visit_expr_quote(self, quote),
        }
    }

    fn visit_expr_op_bin_member(&mut self, member: &'ast ExprOpBinMember) {
        match member {
            ExprOpBinMember::Dot(value) => self.visit_expr(&value.receiver),
            ExprOpBinMember::Optional(value) => self.visit_expr(&value.receiver),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn independent_writers_merge_usage_and_reload_the_same_counts() {
        let path = std::env::temp_dir().join(format!(
            "spectral-m2-completion-{}-{}.json",
            std::process::id(),
            rand::random::<u64>()
        ));
        std::thread::scope(|threads| {
            for _ in 0..4 {
                let path = &path;
                threads.spawn(move || {
                    for _ in 0..10 {
                        record_file(path, &["ideal".to_string()]).unwrap();
                    }
                });
            }
        });
        let usage = read_file(&path).unwrap();
        assert_eq!(usage.count("ideal"), 40);
        assert_eq!(usage.count("image"), 0);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn references_skip_strings_comments_definitions_and_member_names() {
        let mut locals = BTreeSet::new();
        let used = references(
            "answer = ideal(ideal, identity); -- ideal\n\"identity\"; obj.ideal",
            &mut locals,
        );
        assert_eq!(used, ["ideal", "ideal", "identity", "obj"]);
    }

    #[test]
    fn lambda_parameters_and_nested_locals_do_not_count() {
        let mut locals = BTreeSet::new();
        let used = references(
            "f = (ideal, image) -> (identity := ideal; image + identity + rank); ideal",
            &mut locals,
        );
        assert_eq!(used, ["rank", "ideal"]);
        assert!(locals.is_empty());
    }

    #[test]
    fn loop_and_exception_bindings_are_scoped() {
        let mut locals = BTreeSet::new();
        let used = references(
            "for ideal in image list ideal; ideal; try rank except image do image; image",
            &mut locals,
        );
        assert_eq!(used, ["image", "ideal", "rank", "image"]);
        assert!(locals.is_empty());
        assert_eq!(
            references(
                "for ideal in ideal list ideal; for image from 1 to image list image",
                &mut locals
            ),
            ["ideal", "image"]
        );
    }

    #[test]
    fn top_level_locals_remain_local_in_later_cells() {
        let mut locals = BTreeSet::new();
        assert_eq!(
            references(
                "(ideal, image) := (rank, degree); ideal + image",
                &mut locals
            ),
            ["rank", "degree"]
        );
        assert_eq!(references("ideal + image + rank", &mut locals), ["rank"]);
        assert_eq!(
            references("local rank; rank; global ideal", &mut locals),
            ["ideal"]
        );
    }
}
