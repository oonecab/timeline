//! Validated, deterministic graph traversal shared by read-only exporters.

use crate::{Error, Result, TLNode, Timeline};
use std::collections::{HashMap, HashSet};

pub(crate) struct ExportGraph<'a> {
    pub nodes: Vec<&'a TLNode>,
    pub indices: HashMap<&'a str, usize>,
    pub children: Vec<Vec<usize>>,
    pub heads: HashMap<&'a str, Vec<&'a str>>,
    pub head: &'a str,
    pub root: usize,
    pub edges: usize,
}

impl<'a> ExportGraph<'a> {
    pub fn new(timeline: &'a Timeline) -> Result<Self> {
        let mut nodes: Vec<_> = timeline
            .nodes
            .values()
            .chain(timeline.archive.values())
            .collect();
        nodes.sort_by_key(|n| (n.id.len(), n.id.as_str()));
        let indices: HashMap<_, _> = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.id.as_str(), i))
            .collect();
        if indices.len() != nodes.len() {
            return Err(Error::new("cannot export duplicate node IDs".into()));
        }
        let roots: Vec<_> = nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| n.parent.is_none())
            .map(|(i, _)| i)
            .collect();
        if roots.len() != 1 {
            return Err(Error::new(
                "cannot export a timeline without one root".into(),
            ));
        }
        let head = timeline
            .branches
            .get(&timeline.current_branch)
            .ok_or_else(|| Error::new("cannot export a missing current branch".into()))?;
        let current_path: HashSet<_> = timeline.path(head)?.into_iter().collect();
        let mut children = vec![Vec::new(); nodes.len()];
        let mut edges = 0;
        for (i, node) in nodes.iter().enumerate() {
            if let Some(parent) = &node.parent {
                let p = *indices
                    .get(parent.as_str())
                    .ok_or_else(|| Error::new("cannot export a missing parent".into()))?;
                children[p].push(i);
                edges += 1;
            }
            if let Some(range) = &node.summary_range {
                for id in [&range.from, &range.to] {
                    if !indices.contains_key(id.as_str()) {
                        return Err(Error::new(
                            "cannot export a missing summary endpoint".into(),
                        ));
                    }
                }
                edges += if range.from == range.to { 1 } else { 2 };
            }
            if let Some(old) = &node.supersedes {
                if !indices.contains_key(old.as_str()) {
                    return Err(Error::new("cannot export a missing summary version".into()));
                }
                edges += 1;
            }
            for dependency in &node.derived_from {
                if !indices.contains_key(dependency.as_str()) {
                    return Err(Error::new(
                        "cannot export a missing summary dependency".into(),
                    ));
                }
                edges += 1;
            }
        }
        for siblings in &mut children {
            siblings.sort_by_key(|&i| {
                (
                    !current_path.contains(&nodes[i].id),
                    nodes[i].summary_range.is_some(),
                    i,
                )
            });
        }
        let mut seen = vec![false; nodes.len()];
        let mut stack = vec![roots[0]];
        while let Some(i) = stack.pop() {
            if seen[i] {
                return Err(Error::new("cannot export cyclic history".into()));
            }
            seen[i] = true;
            stack.extend(&children[i]);
        }
        if seen.contains(&false) {
            return Err(Error::new("cannot export disconnected history".into()));
        }
        let mut heads: HashMap<&str, Vec<&str>> = HashMap::new();
        for (name, node) in &timeline.branches {
            if !indices.contains_key(node.as_str()) {
                return Err(Error::new("cannot export a missing branch head".into()));
            }
            heads.entry(node).or_default().push(name);
        }
        for names in heads.values_mut() {
            names.sort_unstable();
        }
        Ok(Self {
            nodes,
            indices,
            children,
            heads,
            head,
            root: roots[0],
            edges,
        })
    }
}
