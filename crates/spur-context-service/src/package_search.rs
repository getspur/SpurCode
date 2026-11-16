//! Shared package-name discovery policy for the SQL and Parquet catalogs.

// SOLVE PRE (bd-lah8): accept one edit and three-character terms, reject two
// edits and fuzzy matching of one/two-character terms. The validated Z3 model
// fixes these values; this is deliberately a single-edit matcher.
const MAX_EDITS: usize = 1;
const MIN_FUZZY_CHARS: usize = 3;

pub(crate) struct PackageNameFilter {
    query: Option<String>,
    terms: Vec<String>,
}

impl PackageNameFilter {
    pub(crate) fn new(query: Option<&str>) -> Self {
        let query = query
            .map(str::trim)
            .filter(|query| !query.is_empty())
            .map(str::to_lowercase);
        let terms = query
            .as_deref()
            .map(|query| {
                query
                    .split(|character: char| !character.is_alphanumeric())
                    .filter(|term| !term.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        Self { query, terms }
    }

    pub(crate) fn matches(&self, package: &str) -> bool {
        let Some(query) = self.query.as_deref() else {
            return true;
        };
        // A nonempty punctuation-only filter must not become a wildcard.
        if self.terms.is_empty() {
            return false;
        }
        let package = package.to_lowercase();
        if package.contains(query) {
            return true;
        }
        let components = package
            .split(|character: char| !character.is_alphanumeric())
            .filter(|component| !component.is_empty())
            .collect::<Vec<_>>();
        self.terms.iter().all(|term| {
            package.contains(term)
                || (term.chars().count() >= MIN_FUZZY_CHARS
                    && (within_one_edit(&package, term)
                        || components
                            .iter()
                            .any(|component| within_one_edit(component, term))))
        })
    }
}

/// One Unicode-character insertion, deletion, substitution, or adjacent swap.
fn within_one_edit(left: &str, right: &str) -> bool {
    let left = left.chars().collect::<Vec<_>>();
    let right = right.chars().collect::<Vec<_>>();
    if left.len().abs_diff(right.len()) > MAX_EDITS {
        return false;
    }
    let Some(index) = left
        .iter()
        .zip(&right)
        .position(|(left, right)| left != right)
    else {
        // Equal strings, or a single trailing insertion/deletion.
        return true;
    };
    match left.len().cmp(&right.len()) {
        std::cmp::Ordering::Less => left[index..] == right[index + 1..],
        std::cmp::Ordering::Greater => left[index + 1..] == right[index..],
        std::cmp::Ordering::Equal => {
            left[index + 1..] == right[index + 1..]
                || (index + 1 < left.len()
                    && left[index] == right[index + 1]
                    && left[index + 1] == right[index]
                    && left[index + 2..] == right[index + 2..])
        }
    }
}
