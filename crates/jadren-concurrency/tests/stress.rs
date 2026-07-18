use jadren_concurrency::{
    CancellationToken, DependencyGraph, ScopedTaskJob, TaskGraphBuilder, TaskId, TaskScope,
    TaskSpec, WorkStealingPool,
};
use std::collections::BTreeMap;
use std::env;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

fn iterations() -> usize {
    env::var("JADREN_CONCURRENCY_STRESS_ITERS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
        .unwrap_or(64)
}

fn dependency_graph() -> DependencyGraph {
    let mut builder = TaskGraphBuilder::default();
    builder
        .add_task(TaskSpec::new(TaskId::new(1), "root"))
        .unwrap();
    builder
        .add_task(TaskSpec::new(TaskId::new(2), "left").depends_on(TaskId::new(1)))
        .unwrap();
    builder
        .add_task(TaskSpec::new(TaskId::new(3), "right").depends_on(TaskId::new(1)))
        .unwrap();
    builder
        .add_task(
            TaskSpec::new(TaskId::new(4), "join")
                .depends_on(TaskId::new(2))
                .depends_on(TaskId::new(3)),
        )
        .unwrap();
    builder.build().unwrap()
}

#[test]
fn repeated_work_stealing_graphs_have_one_completion_per_task() {
    for _round in 0..iterations() {
        let graph = dependency_graph();
        let counts = Arc::new((0..4).map(|_| AtomicUsize::new(0)).collect::<Vec<_>>());
        let mut jobs = BTreeMap::new();
        for task in 1..=4 {
            let counts = Arc::clone(&counts);
            jobs.insert(
                TaskId::new(task),
                Box::new(move || {
                    counts[task as usize - 1].fetch_add(1, Ordering::SeqCst);
                    std::thread::yield_now();
                }) as jadren_concurrency::TaskJob,
            );
        }
        let report = WorkStealingPool::new(4)
            .unwrap()
            .execute(&graph, jobs)
            .unwrap();
        assert_eq!(report.completed_order.len(), 4);
        for count in counts.iter() {
            assert_eq!(count.load(Ordering::SeqCst), 1);
        }
        let position = |task: u32| {
            report
                .completed_order
                .iter()
                .position(|completed| completed.value() == task)
                .unwrap()
        };
        assert!(position(1) < position(2));
        assert!(position(1) < position(3));
        assert!(position(2) < position(4));
        assert!(position(3) < position(4));
    }
}

#[test]
fn repeated_scopes_cancel_before_dependent_work() {
    for _round in 0..iterations() {
        let mut builder = TaskGraphBuilder::default();
        builder
            .add_task(TaskSpec::new(TaskId::new(1), "cancel"))
            .unwrap();
        builder
            .add_task(TaskSpec::new(TaskId::new(2), "skipped").depends_on(TaskId::new(1)))
            .unwrap();
        let graph = builder.build().unwrap();
        let executed = Arc::new(AtomicUsize::new(0));
        let mut jobs = BTreeMap::new();
        let first = Arc::clone(&executed);
        jobs.insert(
            TaskId::new(1),
            Box::new(move |token: CancellationToken| {
                first.fetch_add(1, Ordering::SeqCst);
                token.cancel();
            }) as ScopedTaskJob,
        );
        let second = Arc::clone(&executed);
        jobs.insert(
            TaskId::new(2),
            Box::new(move |_token: CancellationToken| {
                second.fetch_add(1, Ordering::SeqCst);
            }) as ScopedTaskJob,
        );
        let scope = TaskScope::new();
        assert!(
            scope
                .run(WorkStealingPool::new(2).unwrap(), &graph, jobs)
                .is_err()
        );
        assert_eq!(executed.load(Ordering::SeqCst), 1);
    }
}
