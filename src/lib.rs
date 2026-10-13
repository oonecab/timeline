//! Pure Timeline ADT. Clocks, file access, and locks live in `storage`.
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

pub mod service;
pub mod storage;

pub const RESERVE: u64 = 32;
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;
pub const MAX_MESSAGES: usize = 100_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl std::error::Error for Error {}
pub type Result<T> = std::result::Result<T, Error>;
fn err<T>(message: &str) -> Result<T> {
    Err(Error(message.into()))
}
fn one() -> u64 {
    1
}
fn discard_derived<'de, D: Deserializer<'de>, T: Default>(
    d: D,
) -> std::result::Result<T, D::Error> {
    let _ = serde::de::IgnoredAny::deserialize(d)?;
    Ok(T::default())
}
pub fn estimate_tokens(text: &str) -> u64 {
    (text.chars().count() as u64).div_ceil(4).max(1)
}
pub fn validate_node_id(id: &str) -> Result<()> {
    if id
        .strip_prefix('n')
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    {
        Ok(())
    } else {
        err("invalid node id: expected n followed by digits")
    }
}
fn validate_branch(name: &str) -> Result<()> {
    if name.is_empty() || name.contains('/') {
        err("branch name must be nonempty and cannot contain /")
    } else {
        Ok(())
    }
}
fn message(role: &str, text: &str, tokens: Option<u64>) -> Result<Message> {
    if !matches!(role, "user" | "assistant" | "system" | "summary") {
        return err("unknown message role");
    }
    if text.len() > MAX_TEXT_BYTES {
        return err("message text exceeds 1 MiB");
    }
    Ok(Message {
        role: role.into(),
        text: text.into(),
        tokens: tokens.unwrap_or_else(|| estimate_tokens(text)),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub text: String,
    pub tokens: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SummaryRange {
    pub from: String,
    pub to: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TLNode {
    pub id: String,
    pub parent: Option<String>,
    pub children: Vec<String>,
    pub message: Option<Message>,
    pub pinned: bool,
    #[serde(default, deserialize_with = "discard_derived")]
    pub prefix_tokens: u64,
    pub summary_range: Option<SummaryRange>,
    #[serde(default = "one")]
    pub summary_version: u64,
    #[serde(default)]
    pub supersedes: Option<String>,
    #[serde(default)]
    pub derived_from: Vec<String>,
    #[serde(default, deserialize_with = "discard_derived")]
    pub stale: bool,
}
impl TLNode {
    fn cost(&self) -> u64 {
        self.message.as_ref().map_or(0, |m| m.tokens)
    }
    pub fn is_placeholder(&self) -> bool {
        self.summary_range.as_ref().is_some_and(|r| {
            self.message.as_ref().is_some_and(|m| {
                m.role == "summary"
                    && m.tokens == RESERVE
                    && m.text.starts_with("[自动摘要占位：")
                    && m.text.ends_with(&format!(
                        " 条消息（{}→{}），调用 summarize 写入真实摘要]",
                        r.from, r.to
                    ))
            })
        })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timeline {
    pub id: String,
    pub nodes: HashMap<String, TLNode>,
    pub branches: HashMap<String, String>,
    pub current_branch: String,
    #[serde(default)]
    pub watermark: Option<String>,
    #[serde(default)]
    pub archive: HashMap<String, TLNode>,
    #[serde(skip)]
    covers: HashMap<String, usize>,
    #[serde(skip)]
    semantic_covers: HashMap<String, usize>,
    #[serde(skip)]
    placeholders: HashMap<SummaryRange, String>,
    #[serde(skip)]
    real_ranges: HashSet<SummaryRange>,
    #[serde(skip)]
    summary_keys: HashMap<String, String>,
    #[serde(skip)]
    successors: HashMap<String, String>,
    #[serde(skip)]
    dependents: HashMap<String, Vec<String>>,
    #[serde(skip)]
    watermark_path: HashSet<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetResult {
    pub messages: Vec<Message>,
    pub total_tokens: u64,
    pub summaries_created: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BranchInfo {
    pub name: String,
    pub head: String,
    pub current: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ArchiveCheck {
    pub eligible: bool,
    pub reasons: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ArchiveResult {
    pub archived: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub ts: u64,
    pub op: String,
    pub timeline_id: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub node: Option<String>,
    #[serde(default)]
    pub details: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdempotencyEntry {
    pub created_at: u64,
    pub result: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Database {
    pub format_version: u64,
    pub next_timeline: u64,
    pub next_node: u64,
    pub timelines: BTreeMap<String, Timeline>,
    #[serde(default)]
    pub audit: Vec<AuditEntry>,
    #[serde(default)]
    pub idempotency: BTreeMap<String, IdempotencyEntry>,
}
impl Default for Database {
    fn default() -> Self {
        Self {
            format_version: 1,
            next_timeline: 0,
            next_node: 0,
            timelines: BTreeMap::new(),
            audit: Vec::new(),
            idempotency: BTreeMap::new(),
        }
    }
}
impl Timeline {
    fn new(id: String, root: String) -> Self {
        let node = TLNode {
            id: root.clone(),
            parent: None,
            children: Vec::new(),
            message: None,
            pinned: false,
            prefix_tokens: 0,
            summary_range: None,
            summary_version: 1,
            supersedes: None,
            derived_from: Vec::new(),
            stale: false,
        };
        Self {
            id,
            nodes: HashMap::from([(root.clone(), node)]),
            branches: HashMap::from([("main".into(), root)]),
            current_branch: "main".into(),
            watermark: None,
            archive: HashMap::new(),
            covers: HashMap::new(),
            semantic_covers: HashMap::new(),
            placeholders: HashMap::new(),
            real_ranges: HashSet::new(),
            summary_keys: HashMap::new(),
            successors: HashMap::new(),
            dependents: HashMap::new(),
            watermark_path: HashSet::new(),
        }
    }
    pub fn node(&self, id: &str) -> Result<&TLNode> {
        validate_node_id(id)?;
        self.nodes
            .get(id)
            .ok_or_else(|| Error("node does not exist".into()))
    }
    fn any_node(&self, id: &str) -> Result<&TLNode> {
        self.nodes
            .get(id)
            .or_else(|| self.archive.get(id))
            .ok_or_else(|| Error("node does not exist".into()))
    }
    pub fn tokens(&self, id: &str) -> Result<u64> {
        Ok(self.node(id)?.prefix_tokens)
    }
    pub fn path(&self, id: &str) -> Result<Vec<String>> {
        self.node(id)?;
        let mut out = Vec::new();
        let mut at = Some(id);
        while let Some(n) = at {
            let node = self.any_node(n)?;
            out.push(n.to_string());
            if out.len() > self.nodes.len() + self.archive.len() {
                return err("parent cycle");
            }
            at = node.parent.as_deref();
        }
        out.reverse();
        Ok(out)
    }
    fn range(&self, from: &str, to: &str) -> Result<Vec<String>> {
        self.node(from)?;
        self.node(to)?;
        self.range_any(from, to)
    }
    fn range_any(&self, from: &str, to: &str) -> Result<Vec<String>> {
        self.any_node(from)?;
        self.any_node(to)?;
        let mut out = Vec::new();
        let mut at = Some(to);
        while let Some(id) = at {
            out.push(id.to_string());
            if id == from {
                out.reverse();
                return Ok(out);
            }
            if out.len() > self.nodes.len() + self.archive.len() {
                break;
            }
            at = self.any_node(id)?.parent.as_deref();
        }
        err("summary range is not continuous: from is not an ancestor of to")
    }
    pub fn context(&self, id: &str) -> Result<Vec<Message>> {
        let path = self.path(id)?;
        if path.len().saturating_sub(1) > MAX_MESSAGES {
            return err("context exceeds 100000 messages; use budget");
        }
        Ok(path
            .iter()
            .filter_map(|id| self.nodes[id].message.clone())
            .collect())
    }
    pub fn fork(&mut self, node: &str, name: &str) -> Result<()> {
        validate_branch(name)?;
        self.node(node)?;
        if self.branches.contains_key(name) {
            return err("branch name already exists");
        }
        self.branches.insert(name.into(), node.into());
        Ok(())
    }
    pub fn switch(&mut self, name: &str) -> Result<()> {
        if !self.branches.contains_key(name) {
            return err("branch does not exist");
        }
        self.current_branch = name.into();
        Ok(())
    }
    pub fn drop_branch(&mut self, name: &str) -> Result<()> {
        if name == self.current_branch {
            return err("cannot delete current branch");
        }
        if self.branches.remove(name).is_none() {
            return err("branch does not exist");
        }
        Ok(())
    }
    pub fn pin(&mut self, id: &str) -> Result<()> {
        self.node(id)?;
        self.nodes.get_mut(id).unwrap().pinned = true;
        Ok(())
    }
    pub fn unpin(&mut self, id: &str) -> Result<()> {
        self.node(id)?;
        if self.watermark_path.contains(id)
            && self.nodes[id].message.is_some()
            && self.covers.get(id).copied().unwrap_or(0) == 0
        {
            return err("cannot unpin: watermark would contain uncovered history");
        }
        self.nodes.get_mut(id).unwrap().pinned = false;
        Ok(())
    }
    pub fn branches(&self) -> Vec<BranchInfo> {
        self.branches
            .iter()
            .map(|(name, head)| BranchInfo {
                name: name.clone(),
                head: head.clone(),
                current: *name == self.current_branch,
            })
            .collect()
    }
    pub fn covers(&self, id: &str) -> usize {
        self.covers.get(id).copied().unwrap_or(0)
    }
    pub fn refcount(&self, id: &str) -> Result<usize> {
        let node = self.node(id)?;
        let mut count = self.covers(id) + usize::from(node.pinned);
        for head in self.branches.values() {
            if self.path(head)?.iter().any(|n| n == id) {
                count += 1;
            }
        }
        Ok(count)
    }
    pub fn archive_check(&self, branch: &str) -> Result<ArchiveCheck> {
        let head = self
            .branches
            .get(branch)
            .ok_or_else(|| Error("branch does not exist".into()))?;
        let path = self.path(head)?;
        let set: HashSet<_> = path.iter().collect();
        let mut reasons = Vec::new();
        if !self.watermark_path.contains(head) {
            reasons.push("watermark: branch head has not passed the watermark".into());
        }
        if path.iter().any(|id| self.nodes[id].pinned) {
            reasons.push("pin: branch path contains pinned nodes".into());
        }
        if self.nodes.values().chain(self.archive.values()).any(|n| {
            n.is_placeholder()
                && n.summary_range.as_ref().is_some_and(|r| {
                    // Since from is an ancestor of to, this interval intersects
                    // a root-to-head path exactly when from is on that path.
                    !self.real_ranges.contains(r) && set.contains(&r.from)
                })
        }) {
            reasons.push("placeholder: branch has unresolved summary dependencies".into());
        }
        Ok(ArchiveCheck {
            eligible: reasons.is_empty(),
            reasons,
        })
    }
    fn advance_watermark(&mut self, to: &str) -> Result<()> {
        let path = self.path(to)?;
        if self.watermark.as_ref().is_none_or(|w| path.contains(w))
            && path.iter().all(|id| {
                self.nodes[id].message.is_none() || self.nodes[id].pinned || self.covers(id) > 0
            })
        {
            self.watermark = Some(to.into());
            self.watermark_path = path.into_iter().collect();
        }
        Ok(())
    }
    fn insert_node(&mut self, node: TLNode) {
        if let Some(parent) = &node.parent {
            self.nodes
                .get_mut(parent)
                .unwrap()
                .children
                .push(node.id.clone());
        }
        self.nodes.insert(node.id.clone(), node);
    }
    fn index_summary(&mut self, id: &str) -> Result<()> {
        let node = self.any_node(id)?.clone();
        let range = node.summary_range.as_ref().unwrap();
        for n in self.range_any(&range.from, &range.to)? {
            *self.covers.entry(n.clone()).or_default() += 1;
            if !node.is_placeholder() {
                *self.semantic_covers.entry(n).or_default() += 1;
            }
        }
        if node.is_placeholder() && self.nodes.contains_key(id) {
            self.placeholders.insert(range.clone(), id.into());
        } else if !node.is_placeholder() {
            self.real_ranges.insert(range.clone());
        }
        let m = node.message.as_ref().unwrap();
        if self.nodes.contains_key(id) {
            self.summary_keys.insert(
                summary_key(range, m, &node.derived_from, node.supersedes.as_deref()),
                id.into(),
            );
        }
        for d in &node.derived_from {
            self.dependents
                .entry(d.clone())
                .or_default()
                .push(id.into());
        }
        if let Some(old) = &node.supersedes {
            self.successors.insert(old.clone(), id.into());
        }
        Ok(())
    }
    fn refresh_stale(&mut self) {
        for n in self.nodes.values_mut().chain(self.archive.values_mut()) {
            n.stale = false;
        }
        let mut stack: Vec<String> = self.successors.keys().cloned().collect();
        let mut seen = HashSet::new();
        while let Some(id) = stack.pop() {
            if !seen.insert(id.clone()) {
                continue;
            }
            if let Some(deps) = self.dependents.get(&id) {
                for d in deps {
                    if let Some(n) = self.nodes.get_mut(d).or_else(|| self.archive.get_mut(d)) {
                        n.stale = true;
                        stack.push(d.clone());
                    }
                }
            }
        }
    }
}
fn summary_key(
    range: &SummaryRange,
    m: &Message,
    derived: &[String],
    supersedes: Option<&str>,
) -> String {
    serde_json::to_string(&(range, m, derived, supersedes)).expect("serializing primitives")
}
impl Database {
    pub fn timeline(&self, id: &str) -> Result<&Timeline> {
        self.timelines
            .get(id)
            .ok_or_else(|| Error("timeline does not exist".into()))
    }
    pub fn timeline_mut(&mut self, id: &str) -> Result<&mut Timeline> {
        self.timelines
            .get_mut(id)
            .ok_or_else(|| Error("timeline does not exist".into()))
    }
    pub fn create(&mut self) -> Result<String> {
        let next_tl = self
            .next_timeline
            .checked_add(1)
            .ok_or_else(|| Error("timeline id counter exhausted".into()))?;
        let next_n = self
            .next_node
            .checked_add(1)
            .ok_or_else(|| Error("node id counter exhausted".into()))?;
        let id = format!("tl{}", self.next_timeline);
        let root = format!("n{}", self.next_node);
        self.timelines
            .insert(id.clone(), Timeline::new(id.clone(), root));
        self.next_timeline = next_tl;
        self.next_node = next_n;
        Ok(id)
    }
    pub fn append(
        &mut self,
        tl: &str,
        role: &str,
        text: &str,
        tokens: Option<u64>,
    ) -> Result<String> {
        let msg = message(role, text, tokens)?;
        let timeline = self.timeline(tl)?;
        let head = timeline.branches[&timeline.current_branch].clone();
        let prefix = timeline
            .node(&head)?
            .prefix_tokens
            .checked_add(msg.tokens)
            .ok_or_else(|| Error("token count overflow".into()))?;
        let next = self
            .next_node
            .checked_add(1)
            .ok_or_else(|| Error("node id counter exhausted".into()))?;
        let id = format!("n{}", self.next_node);
        let node = TLNode {
            id: id.clone(),
            parent: Some(head),
            children: Vec::new(),
            message: Some(msg),
            pinned: false,
            prefix_tokens: prefix,
            summary_range: None,
            summary_version: 1,
            supersedes: None,
            derived_from: Vec::new(),
            stale: false,
        };
        let timeline = self.timeline_mut(tl)?;
        timeline.insert_node(node);
        timeline
            .branches
            .insert(timeline.current_branch.clone(), id.clone());
        self.next_node = next;
        Ok(id)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn summarize(
        &mut self,
        tl: &str,
        from: &str,
        to: &str,
        text: &str,
        tokens: Option<u64>,
        derived_from: Vec<String>,
        supersede: Option<String>,
    ) -> Result<String> {
        let msg = message("summary", text, tokens)?;
        self.add_summary(tl, from, to, msg, derived_from, supersede, false)
    }
    #[allow(clippy::too_many_arguments)]
    fn add_summary(
        &mut self,
        tl: &str,
        from: &str,
        to: &str,
        msg: Message,
        derived_from: Vec<String>,
        supersede: Option<String>,
        placeholder: bool,
    ) -> Result<String> {
        let t = self.timeline(tl)?;
        let range = SummaryRange {
            from: from.into(),
            to: to.into(),
        };
        t.range(from, to)?;
        let parent = t
            .node(from)?
            .parent
            .clone()
            .ok_or_else(|| Error("cannot summarize the root node".into()))?;
        let key = summary_key(&range, &msg, &derived_from, supersede.as_deref());
        if let Some(existing) = t.summary_keys.get(&key) {
            return Ok(existing.clone());
        }
        for d in &derived_from {
            if t.node(d)?.summary_range.is_none() {
                return err("derived_from must reference summary nodes");
            }
        }
        let version = if let Some(old_id) = &supersede {
            let old = t.node(old_id)?;
            if old.summary_range.as_ref() != Some(&range) || old.is_placeholder() {
                return err("supersede requires a real summary of the same range");
            }
            if t.successors.contains_key(old_id) {
                return err("summary already superseded; revise the latest version");
            }
            old.summary_version
                .checked_add(1)
                .ok_or_else(|| Error("summary version overflow".into()))?
        } else {
            1
        };
        let prefix = t
            .node(&parent)?
            .prefix_tokens
            .checked_add(msg.tokens)
            .ok_or_else(|| Error("token count overflow".into()))?;
        let next = self
            .next_node
            .checked_add(1)
            .ok_or_else(|| Error("node id counter exhausted".into()))?;
        let id = format!("n{}", self.next_node);
        let node = TLNode {
            id: id.clone(),
            parent: Some(parent),
            children: Vec::new(),
            message: Some(msg),
            pinned: false,
            prefix_tokens: prefix,
            summary_range: Some(range),
            summary_version: version,
            supersedes: supersede,
            derived_from,
            stale: false,
        };
        let t = self.timeline_mut(tl)?;
        t.insert_node(node);
        t.index_summary(&id)?;
        if t.nodes[&id].supersedes.is_some() {
            t.refresh_stale();
        } else {
            let stale = t.nodes[&id]
                .derived_from
                .iter()
                .any(|d| t.successors.contains_key(d) || t.nodes[d].stale);
            t.nodes.get_mut(&id).unwrap().stale = stale;
        }
        // Budget batches advance once after creating all their placeholders.
        if !placeholder {
            t.advance_watermark(to)?;
        }
        self.next_node = next;
        self.audit.push(AuditEntry {
            ts: 0,
            op: if placeholder {
                "placeholder"
            } else {
                "summarize"
            }
            .into(),
            timeline_id: tl.into(),
            from: Some(from.into()),
            to: Some(to.into()),
            node: Some(id.clone()),
            details: Value::Null,
        });
        Ok(id)
    }
    pub fn archive_branch(&mut self, tl: &str, branch: &str) -> Result<ArchiveResult> {
        let t = self.timeline(tl)?;
        if t.current_branch == branch {
            return err("cannot archive current branch");
        }
        let check = t.archive_check(branch)?;
        if !check.eligible {
            return Err(Error(check.reasons.join("; ")));
        }
        let path = t.path(&t.branches[branch])?;
        let mut shared = HashSet::new();
        for (name, head) in &t.branches {
            if name != branch {
                shared.extend(t.path(head)?);
            }
        }
        let archived: Vec<_> = path
            .into_iter()
            .filter(|id| !shared.contains(id) && !t.nodes[id].pinned && t.covers(id) == 0)
            .collect();
        let t = self.timeline_mut(tl)?;
        for id in &archived {
            let node = t.nodes.remove(id).unwrap();
            t.archive.insert(id.clone(), node);
        }
        t.branches.remove(branch);
        self.audit.push(AuditEntry {
            ts: 0,
            op: "archive_branch".into(),
            timeline_id: tl.into(),
            from: None,
            to: None,
            node: None,
            details: serde_json::json!({"branch":branch,"archived":archived}),
        });
        Ok(ArchiveResult { archived })
    }
}

impl Database {
    pub fn context_budget(
        &mut self,
        tl: &str,
        node: &str,
        max_tokens: u64,
    ) -> Result<BudgetResult> {
        self.context_budget_with_watermark(tl, node, max_tokens, true)
    }
    /// The watermark fast path is deliberately deferred. Both settings run the
    /// same linear selection, making the optional optimization observationally inert.
    pub fn context_budget_with_watermark(
        &mut self,
        tl: &str,
        node: &str,
        max_tokens: u64,
        _enabled: bool,
    ) -> Result<BudgetResult> {
        let t = self.timeline(tl)?;
        let path = t.path(node)?;
        let n = path.len();
        let costs: Vec<u128> = path
            .iter()
            .map(|id| u128::from(t.nodes[id].cost()))
            .collect();
        let mut kept = vec![false; n];
        kept[0] = true;
        kept[n - 1] = true;
        let mut mandatory = kept.clone();
        let mut spent = costs[n - 1];
        let mut pin_total = 0u128;
        for (i, id) in path.iter().enumerate() {
            if t.nodes[id].pinned {
                pin_total += costs[i];
                mandatory[i] = true;
                if !kept[i] {
                    kept[i] = true;
                    spent += costs[i];
                }
            }
        }
        let max = u128::from(max_tokens);
        let bound = max + u128::from(RESERVE);
        // Mechanical placeholders cover history but are not semantic hubs.
        for i in (1..n).rev() {
            if !kept[i]
                && t.semantic_covers.get(&path[i]).copied().unwrap_or(0) > 0
                && spent + costs[i] <= max
            {
                kept[i] = true;
                spent += costs[i];
            }
        }
        let mut in_drop = false;
        for i in (1..n.saturating_sub(1)).rev() {
            if kept[i] {
                in_drop = false;
                continue;
            }
            if spent + costs[i] <= max {
                kept[i] = true;
                spent += costs[i];
                in_drop = false;
            } else if !in_drop {
                spent += u128::from(RESERVE);
                in_drop = true;
            }
        }
        let output_cost = |selection: &[bool]| -> u128 {
            let mut total = 0;
            let mut drop = false;
            for i in 1..n {
                if selection[i] {
                    total += costs[i];
                    drop = false;
                } else if !drop {
                    total += u128::from(RESERVE);
                    drop = true;
                }
            }
            total
        };
        if pin_total <= max && output_cost(&kept) > bound {
            // The published greedy algorithm can overspend on multiple gaps.
            // Minimum feasible layout = mandatory messages + min(raw, reserve)
            // per gap. Then add optional messages by tier using O(1) gap deltas.
            kept = mandatory;
            let mut i = 1;
            while i < n {
                if kept[i] {
                    i += 1;
                    continue;
                }
                let start = i;
                let mut sum = 0;
                while i < n && !kept[i] {
                    sum += costs[i];
                    i += 1;
                }
                if sum <= u128::from(RESERVE) {
                    kept[start..i].fill(true);
                }
            }
            let mut total = output_cost(&kept);
            if total > bound {
                return err(
                    "budget infeasible: target, pins and required summary gaps exceed max_tokens + RESERVE",
                );
            }
            for tier in [2, 3] {
                for i in (1..n).rev() {
                    if kept[i]
                        || (tier == 2 && t.semantic_covers.get(&path[i]).copied().unwrap_or(0) == 0)
                    {
                        continue;
                    }
                    let neighbours =
                        usize::from(i > 1 && !kept[i - 1]) + usize::from(i + 1 < n && !kept[i + 1]);
                    let candidate = total + costs[i] + (neighbours as u128) * u128::from(RESERVE)
                        - u128::from(RESERVE);
                    if candidate <= bound {
                        kept[i] = true;
                        total = candidate;
                    }
                }
            }
        }
        let total = output_cost(&kept);
        let total_tokens =
            u64::try_from(total).map_err(|_| Error("token count overflow".into()))?;
        let mut segments = Vec::new();
        let mut i = 1;
        let mut result_count = 0;
        while i < n {
            result_count += 1;
            if kept[i] {
                i += 1;
            } else {
                let start = i;
                while i < n && !kept[i] {
                    i += 1;
                }
                segments.push((start, i - 1));
            }
        }
        if result_count > MAX_MESSAGES {
            return err("budget result exceeds 100000 messages; reduce budget or pins");
        }
        let mut new_count = 0;
        for &(a, b) in &segments {
            let range = SummaryRange {
                from: path[a].clone(),
                to: path[b].clone(),
            };
            if !t.placeholders.contains_key(&range) {
                t.nodes[&path[a - 1]]
                    .prefix_tokens
                    .checked_add(RESERVE)
                    .ok_or_else(|| Error("token count overflow".into()))?;
                new_count += 1;
            }
        }
        self.next_node
            .checked_add(new_count)
            .ok_or_else(|| Error("node id counter exhausted".into()))?;
        let mut summaries_created = Vec::new();
        let mut replacements = HashMap::new();
        let mut created_ends = HashSet::new();
        for (a, b) in segments {
            let range = SummaryRange {
                from: path[a].clone(),
                to: path[b].clone(),
            };
            let existing = self.timeline(tl)?.placeholders.get(&range).cloned();
            let id = if let Some(id) = existing {
                id
            } else {
                let msg = Message {
                    role: "summary".into(),
                    text: format!(
                        "[自动摘要占位：{} 条消息（{}→{}），调用 summarize 写入真实摘要]",
                        b - a + 1,
                        path[a],
                        path[b]
                    ),
                    tokens: RESERVE,
                };
                let id = self.add_summary(tl, &path[a], &path[b], msg, Vec::new(), None, true)?;
                created_ends.insert(b);
                id
            };
            replacements.insert(a, (b, id.clone()));
            summaries_created.push(id);
        }
        // Preserve the successful early advances even when a later interval is
        // separated from them by a retained, unpinned, uncovered message.
        let mut furthest = None;
        let t = self.timeline(tl)?;
        for (i, id) in path.iter().enumerate().skip(1) {
            if !t.nodes[id].pinned && t.covers(id) == 0 {
                break;
            }
            if created_ends.contains(&i) {
                furthest = Some(id.clone());
            }
        }
        if let Some(to) = furthest {
            self.timeline_mut(tl)?.advance_watermark(&to)?;
        }
        let t = self.timeline(tl)?;
        let mut messages = Vec::with_capacity(result_count);
        let mut i = 1;
        while i < n {
            if kept[i] {
                messages.push(t.nodes[&path[i]].message.clone().unwrap());
                i += 1;
            } else {
                let (end, id) = &replacements[&i];
                messages.push(t.nodes[id].message.clone().unwrap());
                i = end + 1;
            }
        }
        Ok(BudgetResult {
            messages,
            total_tokens,
            summaries_created,
        })
    }
    pub fn rebuild_and_validate(&mut self) -> Result<()> {
        if self.format_version != 1 {
            return err("unsupported format_version; restore the .bak backup");
        }
        let mut global = HashSet::new();
        for (id, t) in &mut self.timelines {
            let number = id
                .strip_prefix("tl")
                .and_then(|s| s.parse::<u64>().ok())
                .ok_or_else(|| Error("invalid timeline id".into()))?;
            if number >= self.next_timeline || id != &t.id {
                return err("timeline counter or identity is invalid");
            }
            for node in t.nodes.keys().chain(t.archive.keys()) {
                validate_node_id(node)?;
                let number = node[1..]
                    .parse::<u64>()
                    .map_err(|_| Error("node id counter overflow".into()))?;
                if number >= self.next_node || !global.insert(node.clone()) {
                    return err("node counter or global identity is invalid");
                }
            }
            t.rebuild()?;
        }
        Ok(())
    }
}

impl Timeline {
    fn rebuild(&mut self) -> Result<()> {
        let mut roots = Vec::new();
        let mut child_references: HashMap<&str, usize> = HashMap::new();
        for (id, n) in self.nodes.iter().chain(self.archive.iter()) {
            validate_node_id(id)?;
            if id != &n.id {
                return err("node identity mismatch");
            }
            if n.parent.is_none() {
                roots.push(id.clone());
                if n.message.is_some() || n.summary_range.is_some() {
                    return err("root must have no message or summary");
                }
            } else {
                if self.nodes.contains_key(id)
                    && !self.nodes.contains_key(n.parent.as_deref().unwrap())
                {
                    return err("active node cannot descend from archived history");
                }
                self.any_node(n.parent.as_deref().unwrap())?;
                let m = n
                    .message
                    .as_ref()
                    .ok_or_else(|| Error("nonroot node has no message".into()))?;
                message(&m.role, &m.text, Some(m.tokens))?;
            }
            let mut unique = HashSet::new();
            for child in &n.children {
                if !unique.insert(child) || self.any_node(child)?.parent.as_deref() != Some(id) {
                    return err("invalid children list");
                }
                *child_references.entry(child).or_default() += 1;
            }
        }
        for (id, n) in self.nodes.iter().chain(self.archive.iter()) {
            if n.parent.is_some() && child_references.get(id.as_str()).copied() != Some(1) {
                return err("parent and children disagree");
            }
        }
        if roots.len() != 1 || !self.nodes.contains_key(&roots[0]) {
            return err("timeline must have exactly one active root");
        }
        let mut queue = vec![(roots[0].clone(), 0u64)];
        let mut prefixes = HashMap::new();
        while let Some((id, parent_cost)) = queue.pop() {
            if prefixes.contains_key(&id) {
                return err("parent cycle");
            }
            let n = self.any_node(&id)?;
            let sum = parent_cost
                .checked_add(n.cost())
                .ok_or_else(|| Error("token count overflow".into()))?;
            prefixes.insert(id.clone(), sum);
            for child in &n.children {
                queue.push((child.clone(), sum));
            }
        }
        if prefixes.len() != self.nodes.len() + self.archive.len() {
            return err("parent cycle or disconnected nodes");
        }
        for (id, cost) in prefixes {
            if let Some(n) = self
                .nodes
                .get_mut(&id)
                .or_else(|| self.archive.get_mut(&id))
            {
                n.prefix_tokens = cost;
                n.stale = false;
            }
        }
        if !self.branches.contains_key(&self.current_branch) {
            return err("current branch does not exist");
        }
        for (name, head) in &self.branches {
            validate_branch(name)?;
            self.node(head)?;
        }
        self.covers.clear();
        self.semantic_covers.clear();
        self.placeholders.clear();
        self.real_ranges.clear();
        self.summary_keys.clear();
        self.successors.clear();
        self.dependents.clear();
        self.watermark_path.clear();
        let mut summaries: Vec<_> = self
            .nodes
            .values()
            .chain(self.archive.values())
            .filter(|n| n.summary_range.is_some())
            .map(|n| n.id.clone())
            .collect();
        summaries.sort_by_key(|id| id[1..].parse::<u64>().unwrap_or(u64::MAX));
        let mut dependency_edges: HashMap<String, Vec<String>> = HashMap::new();
        let mut indegree: HashMap<String, usize> = HashMap::new();
        for n in self.nodes.values().chain(self.archive.values()) {
            if n.summary_range.is_none() && (!n.derived_from.is_empty() || n.supersedes.is_some()) {
                return err("only summaries can have version dependencies");
            }
        }
        for id in &summaries {
            let n = self.any_node(id)?;
            let r = n.summary_range.as_ref().unwrap();
            self.range_any(&r.from, &r.to)?;
            if n.message.as_ref().is_none_or(|m| m.role != "summary")
                || n.parent != self.any_node(&r.from)?.parent
                || n.parent.is_none()
            {
                return err("invalid summary placement or message");
            }
            if n.summary_version == 0 {
                return err("summary version must start at 1");
            }
            indegree.entry(id.clone()).or_default();
            for d in &n.derived_from {
                if self.any_node(d)?.summary_range.is_none() {
                    return err("derived_from must reference summaries");
                }
                dependency_edges
                    .entry(d.clone())
                    .or_default()
                    .push(id.clone());
                *indegree.entry(id.clone()).or_default() += 1;
            }
            if let Some(old) = &n.supersedes {
                let old_node = self.any_node(old)?;
                if old_node.is_placeholder()
                    || old_node.summary_range.as_ref() != Some(r)
                    || old_node.summary_version.checked_add(1) != Some(n.summary_version)
                {
                    return err("invalid summary version chain");
                }
                dependency_edges
                    .entry(old.clone())
                    .or_default()
                    .push(id.clone());
                *indegree.entry(id.clone()).or_default() += 1;
            } else if n.summary_version != 1 {
                return err("first summary version must be 1");
            }
        }
        let mut ready: Vec<_> = indegree
            .iter()
            .filter(|(_, n)| **n == 0)
            .map(|(id, _)| id.clone())
            .collect();
        let mut seen = 0;
        while let Some(id) = ready.pop() {
            seen += 1;
            for child in dependency_edges.get(&id).into_iter().flatten() {
                let degree = indegree.get_mut(child).unwrap();
                *degree -= 1;
                if *degree == 0 {
                    ready.push(child.clone());
                }
            }
        }
        if seen != summaries.len() {
            return err("summary dependency cycle");
        }
        for id in summaries {
            if self
                .any_node(&id)?
                .supersedes
                .as_ref()
                .is_some_and(|old| self.successors.contains_key(old))
            {
                return err("summary version chain cannot fork");
            }
            self.index_summary(&id)?;
        }
        self.refresh_stale();
        if let Some(w) = self.watermark.clone() {
            let path = self.path(&w)?;
            if path.iter().any(|id| {
                self.nodes[id].message.is_some() && !self.nodes[id].pinned && self.covers(id) == 0
            }) {
                return err("watermark contains uncovered history");
            }
            self.watermark_path = path.into_iter().collect();
        }
        Ok(())
    }
    /// Validate source structure and compare cached derived state to a rebuild.
    pub fn validate(&self) -> Result<()> {
        let mut rebuilt = self.clone();
        rebuilt.rebuild()?;
        for (id, n) in self.nodes.iter().chain(self.archive.iter()) {
            let expected = rebuilt.any_node(id)?;
            if n.prefix_tokens != expected.prefix_tokens || n.stale != expected.stale {
                return err("derived node state mismatch");
            }
        }
        if self.covers != rebuilt.covers
            || self.semantic_covers != rebuilt.semantic_covers
            || self.watermark_path != rebuilt.watermark_path
        {
            return err("derived index mismatch");
        }
        Ok(())
    }
}
