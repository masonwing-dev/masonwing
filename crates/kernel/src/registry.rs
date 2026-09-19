use std::collections::{BTreeMap, BTreeSet};

use masonwing_contracts::PluginId;
use thiserror::Error;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DependencyGraph {
    dependencies: BTreeMap<PluginId, Vec<PluginId>>,
}

impl DependencyGraph {
    pub fn insert(&mut self, plugin: PluginId, dependencies: Vec<PluginId>) {
        self.dependencies.insert(plugin, dependencies);
    }

    pub fn resolve(&self, root: &PluginId) -> Result<Vec<PluginId>, RegistryError> {
        let mut visiting = Vec::new();
        let mut visited = BTreeSet::new();
        let mut order = Vec::new();
        self.visit(root, &mut visiting, &mut visited, &mut order)?;
        Ok(order)
    }

    fn visit(
        &self,
        plugin: &PluginId,
        visiting: &mut Vec<PluginId>,
        visited: &mut BTreeSet<PluginId>,
        order: &mut Vec<PluginId>,
    ) -> Result<(), RegistryError> {
        if visited.contains(plugin) {
            return Ok(());
        }
        if let Some(position) = visiting.iter().position(|current| current == plugin) {
            let mut cycle = visiting[position..].to_vec();
            cycle.push(plugin.clone());
            return Err(RegistryError::DependencyCycle { path: cycle });
        }
        let Some(dependencies) = self.dependencies.get(plugin) else {
            return Err(RegistryError::DependencyUnsatisfied {
                plugin: plugin.clone(),
            });
        };

        visiting.push(plugin.clone());
        for dependency in dependencies {
            self.visit(dependency, visiting, visited, order)?;
        }
        visiting.pop();
        visited.insert(plugin.clone());
        order.push(plugin.clone());
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstallRegistry {
    installed: BTreeSet<PluginId>,
}

impl InstallRegistry {
    pub fn install_with_migration(
        &mut self,
        graph: &DependencyGraph,
        root: &PluginId,
        mut migrate: impl FnMut(&PluginId),
    ) -> Result<Vec<PluginId>, RegistryError> {
        let plan = graph.resolve(root)?;
        for plugin in &plan {
            migrate(plugin);
        }
        self.installed.extend(plan.iter().cloned());
        Ok(plan)
    }

    pub fn installed(&self) -> &BTreeSet<PluginId> {
        &self.installed
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum RegistryError {
    #[error("DEPENDENCY_CYCLE")]
    DependencyCycle { path: Vec<PluginId> },
    #[error("DEPENDENCY_UNSATISFIED")]
    DependencyUnsatisfied { plugin: PluginId },
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    // MASONWING@1.0.1 REQ-007 / AC-007 / TC-AC-007.
    #[test]
    fn dependency_cycle_is_rejected_before_migration_or_registry_change() {
        let plugin_a = PluginId::new("plugin_a").unwrap();
        let plugin_b = PluginId::new("plugin_b").unwrap();
        let mut graph = DependencyGraph::default();
        graph.insert(plugin_a.clone(), vec![plugin_b.clone()]);
        graph.insert(plugin_b, vec![plugin_a.clone()]);

        let migration_calls = Cell::new(0_u32);
        let mut registry = InstallRegistry::default();
        let result = registry.install_with_migration(&graph, &plugin_a, |_| {
            migration_calls.set(migration_calls.get() + 1)
        });

        let Err(RegistryError::DependencyCycle { path }) = result else {
            panic!("expected DEPENDENCY_CYCLE");
        };
        assert_eq!(path.first(), path.last());
        assert_eq!(migration_calls.get(), 0);
        assert!(registry.installed().is_empty());
    }

    // MASONWING@1.0.1 REQ-007 / AC-007.
    #[test]
    fn successful_dependency_resolution_installs_in_topological_order() {
        let plugin_a = PluginId::new("plugin_a").unwrap();
        let plugin_b = PluginId::new("plugin_b").unwrap();
        let plugin_c = PluginId::new("plugin_c").unwrap();

        let mut graph = DependencyGraph::default();
        // A depends on B and C, B depends on C (diamond)
        graph.insert(plugin_a.clone(), vec![plugin_b.clone(), plugin_c.clone()]);
        graph.insert(plugin_b.clone(), vec![plugin_c.clone()]);
        graph.insert(plugin_c.clone(), vec![]);

        let migrated = std::cell::RefCell::new(Vec::new());
        let mut registry = InstallRegistry::default();
        let plan = registry
            .install_with_migration(&graph, &plugin_a, |p| {
                migrated.borrow_mut().push(p.clone());
            })
            .expect("acyclic graph resolves");

        assert_eq!(
            plan,
            vec![plugin_c.clone(), plugin_b.clone(), plugin_a.clone()]
        );
        assert_eq!(*migrated.borrow(), plan);
        assert!(registry.installed().contains(&plugin_a));
        assert!(registry.installed().contains(&plugin_b));
        assert!(registry.installed().contains(&plugin_c));
    }

    // MASONWING@1.0.1 REQ-007 / AC-007.
    #[test]
    fn missing_dependency_fails_with_unsatisfied_error() {
        let plugin_a = PluginId::new("plugin_a").unwrap();
        let plugin_missing = PluginId::new("plugin_missing").unwrap();

        let mut graph = DependencyGraph::default();
        graph.insert(plugin_a.clone(), vec![plugin_missing.clone()]);

        let mut registry = InstallRegistry::default();
        let result = registry.install_with_migration(&graph, &plugin_a, |_| {});

        assert_eq!(
            result,
            Err(RegistryError::DependencyUnsatisfied {
                plugin: plugin_missing
            })
        );
        assert!(registry.installed().is_empty());
    }
}
