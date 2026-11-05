//! Commit-graph lane layout.
//!
//! The layout is a pure pass over the loaded commit window. Each row describes
//! which lanes are active at its top and bottom, plus the horizontal edges
//! that connect a merge point or a parent in another lane. The UI paints these
//! primitives directly, so no font glyphs are involved.

use std::collections::HashSet;

use crate::model::CommitSummary;
use crate::{MAX_GRAPH_LANES, Oid};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphEdge {
    pub from: usize,
    pub to: usize,
    pub color: u8,
}

#[derive(Clone, Debug)]
pub struct GraphRow {
    pub node_lane: usize,
    pub lane_count: usize,
    /// Lanes that enter this row from above.
    pub top: Vec<bool>,
    /// Lanes that leave this row toward below.
    pub bottom: Vec<bool>,
    /// Horizontal line segments at the row's vertical center.
    pub edges: Vec<GraphEdge>,
}

impl GraphRow {
    pub fn node_color(&self) -> u8 {
        (self.node_lane % 8) as u8
    }
}

/// Assign lanes for every commit in `commits` (topological order expected).
pub fn layout(commits: &[CommitSummary]) -> Vec<GraphRow> {
    let mut lanes: Vec<Option<Oid>> = Vec::new();
    let mut rows = Vec::with_capacity(commits.len());

    for commit in commits {
        let matching: Vec<usize> = lanes
            .iter()
            .enumerate()
            .filter_map(|(lane, value)| (*value == Some(commit.oid)).then_some(lane))
            .collect();

        let node_lane = matching.first().copied().unwrap_or_else(|| {
            lanes.iter().position(Option::is_none).unwrap_or_else(|| {
                lanes.push(None);
                lanes.len() - 1
            })
        });

        if node_lane >= lanes.len() {
            lanes.resize(node_lane + 1, None);
        }

        let top: Vec<bool> = lanes.iter().map(Option::is_some).collect();

        // A commit reached by several lanes is a merge point: those lanes end here.
        for lane in &matching {
            lanes[*lane] = None;
        }
        lanes[node_lane] = None;

        let mut edges = Vec::new();
        for &merge_lane in &matching {
            if merge_lane != node_lane {
                edges.push(GraphEdge {
                    from: merge_lane,
                    to: node_lane,
                    color: (merge_lane % 8) as u8,
                });
            }
        }

        let mut seen_parents = HashSet::new();
        for (index, parent) in commit.parents.iter().enumerate() {
            if !seen_parents.insert(*parent) {
                continue;
            }

            // The first parent normally continues the commit's own lane. Do
            // not jump to an existing lane here: if another branch already
            // points at the same parent, the two lanes should meet at the
            // parent row, keeping the main line continuous.
            let existing = lanes.iter().position(|value| *value == Some(*parent));
            let target = if index == 0 {
                node_lane
            } else {
                existing.unwrap_or_else(|| {
                    lanes.iter().position(Option::is_none).unwrap_or_else(|| {
                        lanes.push(None);
                        lanes.len() - 1
                    })
                })
            };

            if target >= lanes.len() {
                lanes.resize(target + 1, None);
            }
            lanes[target] = Some(*parent);

            if target != node_lane {
                edges.push(GraphEdge {
                    from: node_lane,
                    to: target,
                    color: (node_lane % 8) as u8,
                });
            }
        }

        // Do not keep trailing empty lanes around: the graph would slowly grow
        // to the right after branches close.
        while matches!(lanes.last(), Some(None)) {
            lanes.pop();
        }

        let bottom: Vec<bool> = lanes.iter().map(Option::is_some).collect();
        let mut lane_count = node_lane + 1;
        lane_count = lane_count.max(top.len()).max(bottom.len());
        for edge in &edges {
            lane_count = lane_count.max(edge.from + 1).max(edge.to + 1);
        }
        lane_count = lane_count.min(MAX_GRAPH_LANES);

        let node_lane = node_lane.min(MAX_GRAPH_LANES.saturating_sub(1));
        let mut top = top;
        let mut bottom = bottom;
        top.resize(lane_count, false);
        bottom.resize(lane_count, false);
        top.truncate(lane_count);
        bottom.truncate(lane_count);
        edges.retain(|edge| edge.from < lane_count && edge.to < lane_count);

        rows.push(GraphRow {
            node_lane,
            lane_count,
            top,
            bottom,
            edges,
        });
    }

    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CommitSummary;

    fn commit(id: u8, parents: &[u8]) -> CommitSummary {
        CommitSummary {
            oid: Oid::from_bytes(&[id; 20]).unwrap(),
            summary: format!("commit {id}"),
            author_name: "Test".into(),
            author_email: "test@example.com".into(),
            author_time: 0,
            committer_time: 0,
            parents: parents
                .iter()
                .map(|id| Oid::from_bytes(&[*id; 20]).unwrap())
                .collect(),
            refs: Vec::new(),
        }
    }

    #[test]
    fn linear_history_stays_in_one_lane() {
        let rows = layout(&[commit(3, &[2]), commit(2, &[1]), commit(1, &[])]);
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|row| row.node_lane == 0));
        assert!(rows.iter().all(|row| row.lane_count == 1));
        assert_eq!(rows[0].top, vec![false]);
        assert_eq!(rows[0].bottom, vec![true]);
        assert_eq!(rows[1].top, vec![true]);
        assert_eq!(rows[1].bottom, vec![true]);
        assert_eq!(rows[2].top, vec![true]);
        assert_eq!(rows[2].bottom, vec![false]);
        assert!(rows.iter().all(|row| row.edges.is_empty()));
    }

    #[test]
    fn branch_and_merge_creates_disconnected_lane_edges() {
        // 4 (merge) -> 2,3; 3 -> 1; 2 -> 1; 1 (root).
        let rows = layout(&[
            commit(4, &[2, 3]),
            commit(3, &[1]),
            commit(2, &[1]),
            commit(1, &[]),
        ]);

        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].node_lane, 0);
        assert_eq!(rows[0].lane_count, 2);
        assert_eq!(rows[0].bottom, vec![true, true]);
        assert!(
            rows[0]
                .edges
                .iter()
                .any(|edge| edge.from == 0 && edge.to == 1)
        );

        assert_eq!(rows[1].node_lane, 1);
        assert_eq!(rows[1].top, vec![true, true]);

        assert_eq!(rows[2].node_lane, 0);
        assert!(rows[2].edges.is_empty());

        assert_eq!(rows[3].node_lane, 0);
        assert_eq!(rows[3].top, vec![true, true]);
        assert!(
            rows[3]
                .edges
                .iter()
                .any(|edge| edge.from == 1 && edge.to == 0)
        );

        // Every row's line graph must be renderable without gaps: a lane that
        // is active at the top must either continue, end in this commit, or be
        // an edge endpoint.
        for row in &rows {
            for (lane, active) in row.top.iter().enumerate() {
                if *active {
                    assert!(
                        row.bottom[lane]
                            || lane == row.node_lane
                            || row
                                .edges
                                .iter()
                                .any(|edge| edge.from == lane || edge.to == lane)
                    );
                }
            }
        }
    }

    #[test]
    fn duplicate_parents_do_not_create_extra_lanes() {
        let rows = layout(&[commit(2, &[1, 1]), commit(1, &[])]);
        assert_eq!(rows[0].lane_count, 1);
        assert!(rows[0].edges.is_empty());
    }

    #[test]
    fn octopus_merge_has_one_edge_per_extra_parent() {
        let rows = layout(&[
            commit(5, &[4, 3, 2]),
            commit(4, &[1]),
            commit(3, &[1]),
            commit(2, &[1]),
            commit(1, &[]),
        ]);
        assert_eq!(rows[0].node_lane, 0);
        assert_eq!(rows[0].edges.len(), 2);
        assert_eq!(rows[0].lane_count, 3);
    }

    #[test]
    fn empty_history_is_empty() {
        assert!(layout(&[]).is_empty());
    }
}
