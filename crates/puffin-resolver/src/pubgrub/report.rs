use std::borrow::Cow;
use std::ops::Bound;

use derivative::Derivative;
use owo_colors::OwoColorize;
use pubgrub::range::Range;
use pubgrub::report::{DerivationTree, Derived, External, ReportFormatter};
use pubgrub::term::Term;
use pubgrub::type_aliases::Map;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::candidate_selector::CandidateSelector;
use crate::prerelease_mode::PreReleaseStrategy;

use super::{PubGrubPackage, PubGrubVersion};

#[derive(Debug)]
pub(crate) struct PubGrubReportFormatter<'a> {
    /// The versions that were available for each package
    pub(crate) available_versions: &'a FxHashMap<PubGrubPackage, Vec<PubGrubVersion>>,
}

impl ReportFormatter<PubGrubPackage, Range<PubGrubVersion>> for PubGrubReportFormatter<'_> {
    type Output = String;

    fn format_external(
        &self,
        external: &External<PubGrubPackage, Range<PubGrubVersion>>,
    ) -> Self::Output {
        match external {
            External::NotRoot(package, version) => {
                format!("we are solving dependencies of {package} {version}")
            }
            External::NoVersions(package, set) => {
                let set = self.simplify_set(set, package);
                if set.as_ref() == &Range::full() {
                    format!("there are no versions of {package}")
                } else if set.as_singleton().is_some() {
                    format!("there is no version of {package}{set}")
                } else {
                    format!(
                        "there are no versions of {}",
                        PackageRange::new(package, &set)
                    )
                }
            }
            External::UnavailableDependencies(package, set) => {
                let set = self.simplify_set(set, package);
                if set.as_ref() == &Range::full() {
                    format!("dependencies of {package} are unavailable")
                } else {
                    format!(
                        "dependencies of {} are unavailable",
                        PackageRange::new(package, &set)
                    )
                }
            }
            External::UnusableDependencies(package, set, reason) => {
                if let Some(reason) = reason {
                    if matches!(package, PubGrubPackage::Root(_)) {
                        format!("{package} dependencies are unusable: {reason}")
                    } else {
                        let set = self.simplify_set(set, package);
                        if set.as_ref() == &Range::full() {
                            format!("dependencies of {package} are unusable: {reason}")
                        } else {
                            format!(
                                "dependencies of {} are unusable: {reason}",
                                PackageRange::new(package, &set)
                            )
                        }
                    }
                } else {
                    let set = self.simplify_set(set, package);
                    if set.as_ref() == &Range::full() {
                        format!("dependencies of {package} are unusable")
                    } else {
                        format!(
                            "dependencies of {} are unusable",
                            PackageRange::new(package, &set)
                        )
                    }
                }
            }
            External::FromDependencyOf(package, package_set, dependency, dependency_set) => {
                let package_set = self.simplify_set(package_set, package);
                let dependency_set = self.simplify_set(dependency_set, dependency);
                if package_set.as_ref() == &Range::full()
                    && dependency_set.as_ref() == &Range::full()
                {
                    format!("{package} depends on {dependency}")
                } else if package_set.as_ref() == &Range::full() {
                    format!(
                        "{package} depends on {}",
                        PackageRange::new(dependency, &dependency_set)
                    )
                } else if dependency_set.as_ref() == &Range::full() {
                    if matches!(package, PubGrubPackage::Root(_)) {
                        // Exclude the dummy version for root packages
                        format!("{package} depends on {dependency}")
                    } else {
                        format!(
                            "{} depends on {dependency}",
                            PackageRange::new(package, &package_set)
                        )
                    }
                } else {
                    if matches!(package, PubGrubPackage::Root(_)) {
                        // Exclude the dummy version for root packages
                        format!(
                            "{package} depends on {}",
                            PackageRange::new(dependency, &dependency_set)
                        )
                    } else {
                        format!(
                            "{} depends on {}",
                            PackageRange::new(package, &package_set),
                            PackageRange::new(dependency, &dependency_set)
                        )
                    }
                }
            }
        }
    }

    /// Try to print terms of an incompatibility in a human-readable way.
    fn format_terms(&self, terms: &Map<PubGrubPackage, Term<Range<PubGrubVersion>>>) -> String {
        let terms_vec: Vec<_> = terms.iter().collect();
        match terms_vec.as_slice() {
            [] | [(PubGrubPackage::Root(_), _)] => "version solving failed".into(),
            [(package @ PubGrubPackage::Package(..), Term::Positive(range))] => {
                let range = range.simplify(
                    self.available_versions
                        .get(package)
                        .unwrap_or(&vec![])
                        .iter(),
                );
                format!("{} is forbidden", PackageRange::new(package, &range))
            }
            [(package @ PubGrubPackage::Package(..), Term::Negative(range))] => {
                let range = range.simplify(
                    self.available_versions
                        .get(package)
                        .unwrap_or(&vec![])
                        .iter(),
                );
                format!("{} is mandatory", PackageRange::new(package, &range))
            }
            [(p1, Term::Positive(r1)), (p2, Term::Negative(r2))] => self.format_external(
                &External::FromDependencyOf((*p1).clone(), r1.clone(), (*p2).clone(), r2.clone()),
            ),
            [(p1, Term::Negative(r1)), (p2, Term::Positive(r2))] => self.format_external(
                &External::FromDependencyOf((*p2).clone(), r2.clone(), (*p1).clone(), r1.clone()),
            ),
            slice => {
                let str_terms: Vec<_> = slice
                    .iter()
                    .map(|(p, t)| format!("{p}{}", PackageTerm::new(p, *t)))
                    .collect();
                str_terms.join(", ") + " are incompatible"
            }
        }
    }

    /// Simplest case, we just combine two external incompatibilities.
    fn explain_both_external(
        &self,
        external1: &External<PubGrubPackage, Range<PubGrubVersion>>,
        external2: &External<PubGrubPackage, Range<PubGrubVersion>>,
        current_terms: &Map<PubGrubPackage, Term<Range<PubGrubVersion>>>,
    ) -> String {
        // TODO: order should be chosen to make it more logical.
        format!(
            "Because {} and {}, {}.",
            self.format_external(external1),
            self.format_external(external2),
            self.format_terms(current_terms)
        )
    }

    /// Both causes have already been explained so we use their refs.
    fn explain_both_ref(
        &self,
        ref_id1: usize,
        derived1: &Derived<PubGrubPackage, Range<PubGrubVersion>>,
        ref_id2: usize,
        derived2: &Derived<PubGrubPackage, Range<PubGrubVersion>>,
        current_terms: &Map<PubGrubPackage, Term<Range<PubGrubVersion>>>,
    ) -> String {
        // TODO: order should be chosen to make it more logical.
        format!(
            "Because {} ({}) and {} ({}), {}.",
            self.format_terms(&derived1.terms),
            ref_id1,
            self.format_terms(&derived2.terms),
            ref_id2,
            self.format_terms(current_terms)
        )
    }

    /// One cause is derived (already explained so one-line),
    /// the other is a one-line external cause,
    /// and finally we conclude with the current incompatibility.
    fn explain_ref_and_external(
        &self,
        ref_id: usize,
        derived: &Derived<PubGrubPackage, Range<PubGrubVersion>>,
        external: &External<PubGrubPackage, Range<PubGrubVersion>>,
        current_terms: &Map<PubGrubPackage, Term<Range<PubGrubVersion>>>,
    ) -> String {
        // TODO: order should be chosen to make it more logical.
        format!(
            "Because {} ({}) and {}, {}.",
            self.format_terms(&derived.terms),
            ref_id,
            self.format_external(external),
            self.format_terms(current_terms)
        )
    }

    /// Add an external cause to the chain of explanations.
    fn and_explain_external(
        &self,
        external: &External<PubGrubPackage, Range<PubGrubVersion>>,
        current_terms: &Map<PubGrubPackage, Term<Range<PubGrubVersion>>>,
    ) -> String {
        format!(
            "And because {}, {}.",
            self.format_external(external),
            self.format_terms(current_terms)
        )
    }

    /// Add an already explained incompat to the chain of explanations.
    fn and_explain_ref(
        &self,
        ref_id: usize,
        derived: &Derived<PubGrubPackage, Range<PubGrubVersion>>,
        current_terms: &Map<PubGrubPackage, Term<Range<PubGrubVersion>>>,
    ) -> String {
        format!(
            "And because {} ({}), {}.",
            self.format_terms(&derived.terms),
            ref_id,
            self.format_terms(current_terms)
        )
    }

    /// Add an already explained incompat to the chain of explanations.
    fn and_explain_prior_and_external(
        &self,
        prior_external: &External<PubGrubPackage, Range<PubGrubVersion>>,
        external: &External<PubGrubPackage, Range<PubGrubVersion>>,
        current_terms: &Map<PubGrubPackage, Term<Range<PubGrubVersion>>>,
    ) -> String {
        format!(
            "And because {} and {}, {}.",
            self.format_external(prior_external),
            self.format_external(external),
            self.format_terms(current_terms)
        )
    }
}

impl PubGrubReportFormatter<'_> {
    /// Simplify a [`Range`] of versions using the available versions for a package.
    fn simplify_set<'a>(
        &self,
        set: &'a Range<PubGrubVersion>,
        package: &PubGrubPackage,
    ) -> Cow<'a, Range<PubGrubVersion>> {
        if set == &Range::full() {
            Cow::Borrowed(set)
        } else {
            Cow::Owned(set.simplify(self.available_versions.get(package).into_iter().flatten()))
        }
    }

    /// Generate the [`PubGrubHints`] for a derivation tree.
    ///
    /// The [`PubGrubHints`] help users resolve errors by providing additional context or modifying
    /// their requirements.
    pub(crate) fn hints(
        &self,
        derivation_tree: &DerivationTree<PubGrubPackage, Range<PubGrubVersion>>,
        selector: &CandidateSelector,
    ) -> FxHashSet<PubGrubHint> {
        /// Returns `true` if pre-releases were allowed for a package.
        fn allowed_prerelease(package: &PubGrubPackage, selector: &CandidateSelector) -> bool {
            match selector.prerelease_strategy() {
                PreReleaseStrategy::Disallow => false,
                PreReleaseStrategy::Allow => true,
                PreReleaseStrategy::IfNecessary => false,
                PreReleaseStrategy::Explicit(packages) => {
                    if let PubGrubPackage::Package(package, ..) = package {
                        packages.contains(package)
                    } else {
                        false
                    }
                }
                PreReleaseStrategy::IfNecessaryOrExplicit(packages) => {
                    if let PubGrubPackage::Package(package, ..) = package {
                        packages.contains(package)
                    } else {
                        false
                    }
                }
            }
        }

        let mut hints = FxHashSet::default();
        match derivation_tree {
            DerivationTree::External(external) => match external {
                External::NoVersions(package, set) => {
                    if set.bounds().any(PubGrubVersion::any_prerelease) {
                        // A pre-release marker appeared in the version requirements.
                        if !allowed_prerelease(package, selector) {
                            hints.insert(PubGrubHint::PreReleaseRequested {
                                package: package.clone(),
                                range: self.simplify_set(set, package).into_owned(),
                            });
                        }
                    } else if let Some(version) =
                        self.available_versions.get(package).and_then(|versions| {
                            versions
                                .iter()
                                .rev()
                                .filter(|version| version.any_prerelease())
                                .find(|version| set.contains(version))
                        })
                    {
                        // There are pre-release versions available for the package.
                        if !allowed_prerelease(package, selector) {
                            hints.insert(PubGrubHint::PreReleaseAvailable {
                                package: package.clone(),
                                version: version.clone(),
                            });
                        }
                    }
                }
                External::NotRoot(..) => {}
                External::UnavailableDependencies(..) => {}
                External::UnusableDependencies(..) => {}
                External::FromDependencyOf(..) => {}
            },
            DerivationTree::Derived(derived) => {
                hints.extend(self.hints(&derived.cause1, selector));
                hints.extend(self.hints(&derived.cause2, selector));
            }
        }
        hints
    }
}

#[derive(Derivative, Debug, Clone)]
#[derivative(Hash, PartialEq, Eq)]
pub(crate) enum PubGrubHint {
    /// There are pre-release versions available for a package, but pre-releases weren't enabled
    /// for that package.
    ///
    PreReleaseAvailable {
        package: PubGrubPackage,
        #[derivative(PartialEq = "ignore", Hash = "ignore")]
        version: PubGrubVersion,
    },
    /// A requirement included a pre-release marker, but pre-releases weren't enabled for that
    /// package.
    PreReleaseRequested {
        package: PubGrubPackage,
        #[derivative(PartialEq = "ignore", Hash = "ignore")]
        range: Range<PubGrubVersion>,
    },
}

impl std::fmt::Display for PubGrubHint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PubGrubHint::PreReleaseAvailable { package, version } => {
                write!(
                    f,
                    "{}{} Pre-releases are available for {} in the requested range (e.g., {}), but pre-releases weren't enabled (try: `--prerelease=allow`)",
                    "hint".bold().cyan(),
                    ":".bold(),
                    package.bold(),
                    version.bold()
                )
            }
            PubGrubHint::PreReleaseRequested { package, range } => {
                write!(
                    f,
                    "{}{} {} was requested with a pre-release marker (e.g., {}), but pre-releases weren't enabled (try: `--prerelease=allow`)",
                    "hint".bold().cyan(),
                    ":".bold(),
                    package.bold(),
                    PackageRange::new(package, range).bold()
                )
            }
        }
    }
}

/// A derivative of [Term] with custom formatting.
struct PackageTerm<'a> {
    package: &'a PubGrubPackage,
    term: &'a Term<Range<PubGrubVersion>>,
}

impl std::fmt::Display for PackageTerm<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.term {
            Term::Positive(set) => write!(f, "{set}"),
            Term::Negative(set) => {
                if let Some(version) = set.as_singleton() {
                    write!(f, "!={version}")
                } else {
                    write!(f, "!( {} )", PackageRange::new(self.package, set))
                }
            }
        }
    }
}

impl PackageTerm<'_> {
    fn new<'a>(
        package: &'a PubGrubPackage,
        term: &'a Term<Range<PubGrubVersion>>,
    ) -> PackageTerm<'a> {
        PackageTerm { package, term }
    }
}

/// A derivative of [Range] with custom formatting.
struct PackageRange<'a> {
    package: &'a PubGrubPackage,
    range: &'a Range<PubGrubVersion>,
}

impl std::fmt::Display for PackageRange<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.range.is_empty() {
            write!(f, "∅")?;
        } else {
            for (idx, segment) in self.range.iter().enumerate() {
                if idx > 0 {
                    write!(f, "|")?;
                }
                write!(f, "{}", self.package)?;
                match segment {
                    (Bound::Unbounded, Bound::Unbounded) => write!(f, "*")?,
                    (Bound::Unbounded, Bound::Included(v)) => write!(f, "<={v}")?,
                    (Bound::Unbounded, Bound::Excluded(v)) => write!(f, "<{v}")?,
                    (Bound::Included(v), Bound::Unbounded) => write!(f, ">={v}")?,
                    (Bound::Included(v), Bound::Included(b)) => {
                        if v == b {
                            write!(f, "=={v}")?;
                        } else {
                            write!(f, ">={v},<={b}")?;
                        }
                    }
                    (Bound::Included(v), Bound::Excluded(b)) => write!(f, ">={v},<{b}")?,
                    (Bound::Excluded(v), Bound::Unbounded) => write!(f, ">{v}")?,
                    (Bound::Excluded(v), Bound::Included(b)) => write!(f, ">{v},<={b}")?,
                    (Bound::Excluded(v), Bound::Excluded(b)) => write!(f, ">{v},<{b}")?,
                };
            }
        }
        Ok(())
    }
}

impl PackageRange<'_> {
    fn new<'a>(package: &'a PubGrubPackage, range: &'a Range<PubGrubVersion>) -> PackageRange<'a> {
        PackageRange { package, range }
    }
}
