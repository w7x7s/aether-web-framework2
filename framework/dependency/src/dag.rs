//! # Precompiled Topological DAG Dependency Graph
//!
//! Replaces Python FastAPI runtime reflection with a startup-compiled topological sort.
//! Verifies no circular references exist before listening on port 3000.

use crate::scope::DependencyScope;
use aether_core::error::AetherError;
use std::collections::HashMap;

pub type NodeId = usize;

#[derive(Clone, Debug)]
pub struct DependencyNode {
    pub id: NodeId,
    pub name: &'static str,
    pub scope: DependencyScope,
    pub dependencies: Vec<NodeId>,
}

pub struct DependencyGraph {
    nodes: HashMap<NodeId, DependencyNode>,
}

impl DependencyGraph {
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
        }
    }

    pub fn add_node(&mut self, node: DependencyNode) {
        self.nodes.insert(node.id, node);
    }

    /// Compiles a topological execution plan. Errors out on cycles.
    pub fn compile_execution_plan(&self) -> Result<Vec<NodeId>, AetherError> {
        let mut in_degree: HashMap<NodeId, usize> = HashMap::new();
        let mut adj: HashMap<NodeId, Vec<NodeId>> = HashMap::new();

        for (&id, _) in &self.nodes {
            in_degree.insert(id, 0);
            adj.insert(id, Vec::new());
        }

        for (&id, node) in &self.nodes {
            for &dep in &node.dependencies {
                adj.entry(dep).or_default().push(id);
                *in_degree.entry(id).or_default() += 1;
            }
        }

        let mut queue: Vec<NodeId> = in_degree
            .iter()
            .filter(|&(_, &deg)| deg == 0)
            .map(|(&id, _)| id)
            .collect();

        let mut plan = Vec::with_capacity(self.nodes.len());

        while let Some(curr) = queue.pop() {
            plan.push(curr);

            if let Some(neighbors) = adj.get(&curr) {
                for &neighbor in neighbors {
                    let deg = in_degree.get_mut(&neighbor).unwrap();
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push(neighbor);
                    }
                }
            }
        }

        if plan.len() != self.nodes.len() {
            return Err(AetherError::DependencyCycleDetected(
                "Circular dependency detected in graph".into(),
            ));
        }

        Ok(plan)
    }
}

impl Default for DependencyGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_topological_sort_and_cycle_check() {
        let mut graph = DependencyGraph::new();
        // 0: DB Pool (Singleton)
        // 1: Auth Service (Singleton, depends on 0)
        // 2: Current User (RequestScoped, depends on 1)
        graph.add_node(DependencyNode {
            id: 0,
            name: "DbPool",
            scope: DependencyScope::Singleton,
            dependencies: vec![],
        });
        graph.add_node(DependencyNode {
            id: 1,
            name: "AuthService",
            scope: DependencyScope::Singleton,
            dependencies: vec![0],
        });
        graph.add_node(DependencyNode {
            id: 2,
            name: "CurrentUser",
            scope: DependencyScope::RequestScoped,
            dependencies: vec![1],
        });

        let plan = graph.compile_execution_plan().expect("Compilation succeeds");
        assert_eq!(plan, vec![0, 1, 2]);
    }
}
