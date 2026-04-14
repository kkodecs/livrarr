//! Reconciliation engine — clusters hypotheses, scores, merges supplementary fields.

use super::m4_scoring;
use super::types::{Confidence, Extraction, ExtractionSource};

/// A cluster of agreeing extractions, with a merged result.
#[derive(Debug)]
pub struct Cluster {
    /// The primary (title, author) pair — from the highest-ranked hypothesis.
    pub primary: Extraction,
    /// All hypotheses in this cluster.
    pub members: Vec<Extraction>,
    /// Extraction confidence based on agreement and completeness.
    pub confidence: Confidence,
}

/// Reconcile multiple extraction hypotheses into ranked clusters.
/// Returns 1–N clusters, most confident first.
pub fn reconcile(mut extractions: Vec<Extraction>) -> Vec<Cluster> {
    if extractions.is_empty() {
        return vec![];
    }

    if extractions.len() == 1 {
        let e = extractions.remove(0);
        let conf = e.confidence;
        return vec![Cluster {
            primary: e.clone(),
            members: vec![e],
            confidence: conf,
        }];
    }

    // Step 1: Cluster by normalized title+author similarity using union-find
    // for transitive closure (A≈B and B≈C → A,B,C in same cluster).
    let n = extractions.len();
    let mut parent: Vec<usize> = (0..n).collect();

    // Find with iterative path compression.
    fn find(parent: &mut [usize], i: usize) -> usize {
        // Walk to the root, collecting the path.
        let mut root = i;
        while parent[root] != root {
            root = parent[root];
        }
        // Compress: point every node on the path directly at the root.
        let mut cur = i;
        while parent[cur] != root {
            let next = parent[cur];
            parent[cur] = root;
            cur = next;
        }
        root
    }

    // Build agreement graph via union-find.
    for i in 0..n {
        for j in (i + 1)..n {
            if extractions_agree(&extractions[i], &extractions[j]) {
                let ri = find(&mut parent, i);
                let rj = find(&mut parent, j);
                if ri != rj {
                    parent[ri] = rj;
                }
            }
        }
    }

    // Collect connected components.
    let mut cluster_map: std::collections::HashMap<usize, Vec<usize>> =
        std::collections::HashMap::new();
    for i in 0..n {
        let root = find(&mut parent, i);
        cluster_map.entry(root).or_default().push(i);
    }
    let clusters: Vec<Vec<usize>> = cluster_map.into_values().collect();

    // Step 2: For each cluster, pick primary and merge supplementary fields.
    let mut result: Vec<Cluster> = clusters
        .into_iter()
        .filter_map(|indices| {
            let members: Vec<Extraction> =
                indices.iter().map(|&i| extractions[i].clone()).collect();
            build_cluster(members)
        })
        .collect();

    // Step 3: Sort by confidence then completeness.
    result.sort_by(|a, b| {
        b.confidence
            .cmp(&a.confidence)
            .then_with(|| b.primary.completeness().cmp(&a.primary.completeness()))
    });

    result
}

/// Generate synthetic (combinatorial fallback) extractions from a set of existing extractions.
/// Takes all unique valid titles and authors, creates Cartesian product.
pub fn generate_synthetic(extractions: &[Extraction]) -> Vec<Extraction> {
    let titles: Vec<&str> = extractions
        .iter()
        .filter_map(|e| e.title.as_deref())
        .filter(|t| !t.is_empty())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let authors: Vec<&str> = extractions
        .iter()
        .filter_map(|e| e.author.as_deref())
        .filter(|a| !a.is_empty())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    if titles.is_empty() || authors.is_empty() {
        return vec![];
    }

    let mut synthetic = vec![];
    for title in &titles {
        for author in &authors {
            // Skip pairs that already exist as original hypotheses.
            let already_exists = extractions.iter().any(|e| {
                e.title.as_deref() == Some(*title) && e.author.as_deref() == Some(*author)
            });
            if already_exists {
                continue;
            }

            synthetic.push(Extraction {
                title: Some(title.to_string()),
                author: Some(author.to_string()),
                year: None,
                isbn: None,
                language: None,
                series: None,
                series_position: None,
                narrator: None,
                asin: None,
                confidence: Confidence::Low,
                source: ExtractionSource::Synthetic,
            });
        }
    }

    synthetic
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Check if two extractions agree (title_sim > 0.80 AND author_sim > 0.80 or either author missing).
fn extractions_agree(a: &Extraction, b: &Extraction) -> bool {
    let title_a = a.title.as_deref().unwrap_or("");
    let title_b = b.title.as_deref().unwrap_or("");

    if title_a.is_empty() || title_b.is_empty() {
        return false;
    }

    let title_sim = m4_scoring::string_similarity(title_a, title_b);
    if title_sim < 0.80 {
        return false;
    }

    let author_a = a.author.as_deref();
    let author_b = b.author.as_deref();

    match (author_a, author_b) {
        (Some(aa), Some(bb)) if !aa.is_empty() && !bb.is_empty() => {
            m4_scoring::author_similarity(aa, bb) > 0.80
        }
        // If either author is missing, agreement on title is enough.
        _ => true,
    }
}

/// Build a cluster from member extractions.
fn build_cluster(members: Vec<Extraction>) -> Option<Cluster> {
    if members.is_empty() {
        return None;
    }

    // Pick primary: highest completeness, then source trust (Embedded > Path > String).
    let primary_idx = members
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| {
            a.completeness()
                .cmp(&b.completeness())
                .then_with(|| source_trust(a.source).cmp(&source_trust(b.source)))
        })
        .map(|(i, _)| i)
        .unwrap_or(0);

    let mut primary = members[primary_idx].clone();

    // Merge supplementary fields from other members (don't override primary title/author).
    for member in &members {
        if primary.year.is_none() {
            primary.year = member.year;
        }
        if primary.isbn.is_none() {
            primary.isbn.clone_from(&member.isbn);
        }
        if primary.language.is_none() {
            primary.language.clone_from(&member.language);
        }
        if primary.series.is_none() {
            primary.series.clone_from(&member.series);
        }
        if primary.series_position.is_none() {
            primary.series_position = member.series_position;
        }
        if primary.narrator.is_none() {
            primary.narrator.clone_from(&member.narrator);
        }
        if primary.asin.is_none() {
            primary.asin.clone_from(&member.asin);
        }
    }

    // Compute cluster confidence from agreement.
    let agreement_count = members.len();
    let confidence =
        if agreement_count >= 3 || (agreement_count == 2 && primary.has_title_and_author()) {
            Confidence::High
        } else if primary.has_title_and_author() {
            primary.confidence
        } else if primary.has_title() {
            Confidence::MediumLow
        } else {
            Confidence::Low
        };

    Some(Cluster {
        primary,
        members,
        confidence,
    })
}

fn source_trust(source: ExtractionSource) -> u8 {
    match source {
        ExtractionSource::Embedded => 3,
        ExtractionSource::Path => 2,
        ExtractionSource::String => 1,
        ExtractionSource::Synthetic => 0,
    }
}
