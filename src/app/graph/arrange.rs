//! Deterministic layered layout. Nodes are assigned columns so that a node
//! always sits left of everything it feeds, then levelled vertically to keep
//! links horizontal, one connected component per band.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use eframe::egui::{Pos2, Vec2, pos2};

use super::model::{GRID_STEP, GraphModel, NODE_WIDTH};

const COLUMN_GAP: f32 = GRID_STEP;
const ROW_GAP: f32 = 30.0;
const BAND_GAP: f32 = 160.0;
const STRAY_COLUMNS: usize = 5;
const LEVELLING_PASSES: usize = 12;

pub fn arrange(model: &GraphModel) -> Vec<Pos2> {
    let mut positions = vec![Pos2::ZERO; model.nodes.len()];
    if model.nodes.is_empty() {
        return positions;
    }

    let dag = Dag::build(model);
    let columns = dag.columns();
    let column_x = (0..=columns.iter().copied().max().unwrap_or(0))
        .map(|column| column as f32 * (NODE_WIDTH + COLUMN_GAP) + NODE_WIDTH / 2.0)
        .collect::<Vec<_>>();
    let anchors = anchors(model);

    let mut components: Vec<Vec<usize>> = vec![Vec::new(); model.components];
    for (index, node) in model.nodes.iter().enumerate() {
        components[node.component].push(index);
    }
    let (mut clusters, strays): (Vec<_>, Vec<_>) = components
        .into_iter()
        .partition(|members| members.len() > 1);
    clusters.sort_by_key(|members| Reverse(members.len()));

    let mut band_top = 0.0;
    for cluster in &clusters {
        band_top = place_cluster(
            model,
            cluster,
            &columns,
            &column_x,
            &anchors,
            band_top,
            &mut positions,
        );
    }
    place_strays(model, &strays, band_top, &mut positions);

    let center = positions
        .iter()
        .fold(Vec2::ZERO, |acc, pos| acc + pos.to_vec2())
        / positions.len() as f32;
    for pos in &mut positions {
        *pos -= center;
    }
    positions
}

struct Dag {
    order: Vec<usize>,
    outgoing: Vec<Vec<usize>>,
    incoming: Vec<Vec<usize>>,
}

impl Dag {
    fn build(model: &GraphModel) -> Self {
        let count = model.nodes.len();
        let mut outgoing = vec![Vec::new(); count];
        let mut incoming = vec![Vec::new(); count];
        for link in &model.links {
            if link.from_node == link.to_node {
                continue;
            }
            outgoing[link.from_node].push(link.to_node);
            incoming[link.to_node].push(link.from_node);
        }
        for list in outgoing.iter_mut().chain(incoming.iter_mut()) {
            list.sort_unstable();
            list.dedup();
        }

        let mut remaining = incoming.iter().map(Vec::len).collect::<Vec<_>>();
        let mut order = Vec::with_capacity(count);
        let mut placed = vec![false; count];
        let mut queued = vec![false; count];
        let mut broken: Vec<(usize, usize)> = Vec::new();

        // Lowest index first keeps the result deterministic.
        let mut ready = BinaryHeap::new();
        for index in 0..count {
            if remaining[index] == 0 {
                ready.push(Reverse(index));
                queued[index] = true;
            }
        }

        while order.len() < count {
            while let Some(Reverse(node)) = ready.pop() {
                order.push(node);
                placed[node] = true;
                for &next in &outgoing[node] {
                    if queued[next] {
                        continue;
                    }
                    remaining[next] = remaining[next].saturating_sub(1);
                    if remaining[next] == 0 {
                        ready.push(Reverse(next));
                        queued[next] = true;
                    }
                }
            }

            if order.len() == count {
                break;
            }

            // Everything left is in a cycle: free the closest to ready.
            let stuck = (0..count)
                .filter(|&index| !placed[index])
                .min_by_key(|&index| (remaining[index], index))
                .expect("nodes remain but none are unplaced");
            for &previous in &incoming[stuck] {
                if !placed[previous] {
                    broken.push((previous, stuck));
                }
            }
            remaining[stuck] = 0;
            ready.push(Reverse(stuck));
            queued[stuck] = true;
        }

        for (from, to) in broken {
            outgoing[from].retain(|&node| node != to);
            incoming[to].retain(|&node| node != from);
        }

        Self {
            order,
            outgoing,
            incoming,
        }
    }

    fn columns(&self) -> Vec<usize> {
        let mut columns = vec![0usize; self.order.len()];

        for &node in &self.order {
            for &previous in &self.incoming[node] {
                columns[node] = columns[node].max(columns[previous] + 1);
            }
        }

        // Slide right so chains compact against whatever they feed.
        for &node in self.order.iter().rev() {
            let Some(first_output) = self.outgoing[node].iter().map(|&to| columns[to]).min() else {
                continue;
            };
            let earliest = self.incoming[node]
                .iter()
                .map(|&from| columns[from] + 1)
                .max()
                .unwrap_or(0);
            columns[node] = (first_output - 1).max(earliest);
        }

        columns
    }
}

/// One end of a link. Port rows are baked in as offsets, so the layout lines up
/// the link itself rather than the two node centres.
struct Anchor {
    other: usize,
    own: f32,
    theirs: f32,
    incoming: bool,
}

fn anchors(model: &GraphModel) -> Vec<Vec<Anchor>> {
    let mut anchors = Vec::from_iter((0..model.nodes.len()).map(|_| Vec::new()));
    for link in &model.links {
        if link.from_node == link.to_node {
            continue;
        }
        let from = model.nodes[link.from_node].port_offset(link.from_port);
        let to = model.nodes[link.to_node].port_offset(link.to_port);
        anchors[link.from_node].push(Anchor {
            other: link.to_node,
            own: from,
            theirs: to,
            incoming: false,
        });
        anchors[link.to_node].push(Anchor {
            other: link.from_node,
            own: to,
            theirs: from,
            incoming: true,
        });
    }
    anchors
}

/// Lay out one component in its own band, returning the next band's top.
fn place_cluster(
    model: &GraphModel,
    cluster: &[usize],
    columns: &[usize],
    column_x: &[f32],
    anchors: &[Vec<Anchor>],
    band_top: f32,
    positions: &mut [Pos2],
) -> f32 {
    let mut rows: Vec<Vec<usize>> = vec![Vec::new(); column_x.len()];
    for &index in cluster {
        rows[columns[index]].push(index);
    }

    let mut y = vec![0.0f32; model.nodes.len()];
    for column in &rows {
        let mut cursor = 0.0;
        for &index in column {
            y[index] = cursor + model.nodes[index].size.y / 2.0;
            cursor += model.nodes[index].size.y + ROW_GAP;
        }
    }

    let mut best = (link_offset(cluster, anchors, &y), rows.clone(), y.clone());
    for pass in 0..LEVELLING_PASSES {
        // Pulled by inputs on one pass, by outputs on the next.
        let forwards = pass % 2 == 0;
        for step in 0..rows.len() {
            let column = if forwards {
                step
            } else {
                rows.len() - 1 - step
            };
            level_column(model, &mut rows[column], anchors, &mut y, forwards);
        }

        let score = link_offset(cluster, anchors, &y);
        if score < best.0 {
            best = (score, rows.clone(), y.clone());
        }
    }
    let (_, rows, y) = best;

    let extent = |pick: fn(f32, f32) -> f32, sign: f32, seed: f32| {
        cluster
            .iter()
            .map(|&index| y[index] + sign * model.nodes[index].size.y / 2.0)
            .fold(seed, pick)
    };
    let top = extent(f32::min, -1.0, f32::MAX);
    let bottom = extent(f32::max, 1.0, f32::MIN);

    for (column, members) in rows.iter().enumerate() {
        for &index in members {
            positions[index] = pos2(column_x[column], band_top + y[index] - top);
        }
    }

    band_top + (bottom - top) + BAND_GAP
}

/// Pull one column to where its links want it, then re-sort and re-pack. Order is
/// otherwise free, and levelling is also what removes crossings: two links only
/// cross when one runs against the order.
fn level_column(
    model: &GraphModel,
    column: &mut Vec<usize>,
    anchors: &[Vec<Anchor>],
    y: &mut [f32],
    forwards: bool,
) {
    if column.is_empty() {
        return;
    }

    let mut wanted = column
        .iter()
        .map(|&index| {
            // A node with nothing on the swept side follows the other one.
            let mut side = anchors[index]
                .iter()
                .filter(|anchor| anchor.incoming == forwards)
                .peekable();
            let level = |anchor: &Anchor| y[anchor.other] + anchor.theirs - anchor.own;
            let mean = if side.peek().is_some() {
                mean(side.map(level))
            } else {
                mean(anchors[index].iter().map(level))
            };
            (index, mean.unwrap_or(y[index]))
        })
        .collect::<Vec<_>>();

    wanted.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
    *column = wanted.iter().map(|(index, _)| *index).collect();

    // Settling down and settling up are both feasible and keep the order, so the
    // average is feasible too — and closer to what the links asked for.
    let count = column.len();
    let half = |index: usize| model.nodes[index].size.y / 2.0;

    let mut downwards = vec![0.0f32; count];
    downwards[0] = wanted[0].1;
    for slot in 1..count {
        let floor = downwards[slot - 1] + half(column[slot - 1]) + ROW_GAP + half(column[slot]);
        downwards[slot] = wanted[slot].1.max(floor);
    }

    let mut upwards = vec![0.0f32; count];
    upwards[count - 1] = wanted[count - 1].1;
    for slot in (0..count - 1).rev() {
        let ceiling = upwards[slot + 1] - half(column[slot + 1]) - ROW_GAP - half(column[slot]);
        upwards[slot] = wanted[slot].1.min(ceiling);
    }

    for slot in 0..count {
        y[column[slot]] = (downwards[slot] + upwards[slot]) / 2.0;
    }
}

fn mean(values: impl Iterator<Item = f32>) -> Option<f32> {
    let (sum, count) = values.fold((0.0, 0), |(sum, count), value| (sum + value, count + 1));
    (count > 0).then(|| sum / count as f32)
}

/// Total vertical distance the links cover: what levelling minimises.
fn link_offset(cluster: &[usize], anchors: &[Vec<Anchor>], y: &[f32]) -> f32 {
    cluster
        .iter()
        .flat_map(|&index| anchors[index].iter().map(move |anchor| (index, anchor)))
        .filter(|(_, anchor)| !anchor.incoming)
        .map(|(index, anchor)| ((y[index] + anchor.own) - (y[anchor.other] + anchor.theirs)).abs())
        .sum()
}

/// Park the nodes that link to nothing in a grid below the real graph.
fn place_strays(model: &GraphModel, strays: &[Vec<usize>], band_top: f32, positions: &mut [Pos2]) {
    let indices = strays.iter().flatten().copied().collect::<Vec<_>>();
    let cell = indices
        .iter()
        .fold(Vec2::ZERO, |acc, &index| acc.max(model.nodes[index].size))
        + Vec2::new(COLUMN_GAP, ROW_GAP);

    for (position, &index) in indices.iter().enumerate() {
        positions[index] = pos2(
            cell.x * ((position % STRAY_COLUMNS) as f32 + 0.5),
            band_top + cell.y * ((position / STRAY_COLUMNS) as f32 + 0.5),
        );
    }
}
