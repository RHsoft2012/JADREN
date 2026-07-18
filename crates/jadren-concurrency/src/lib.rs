//! Deterministic task dependency graph for Jadren structured concurrency.
//!
//! The graph is deliberately independent from scheduling policy. It proves
//! which tasks may run after which other tasks; the fixed worker pool consumes
//! this contract without weakening it. Cancellation, stealing and host
//! scheduler coexistence remain later layers.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::error::Error;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

/// Stable task identity supplied by the compiler or host scheduler.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TaskId(u32);

impl TaskId {
    /// Creates one stable task identity.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    /// Returns the numeric identity.
    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }
}

/// Stable logical resource name used for conflict analysis.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ResourceId(String);

impl ResourceId {
    /// Creates a resource identity from a canonical non-empty path/name.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the canonical resource spelling.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ResourceId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

/// Access mode used to derive data-conflict edges.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AccessMode {
    /// Immutable access; two reads may overlap.
    Read,
    /// Mutable access; conflicts with every other access to the resource.
    Write,
    /// Synchronization access; serialized conservatively for deterministic order.
    Atomic,
}

impl AccessMode {
    /// Returns whether two accesses require a dependency edge.
    #[must_use]
    pub const fn conflicts(self, other: Self) -> bool {
        !matches!((self, other), (Self::Read, Self::Read))
    }
}

/// One task/resource access declaration.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceAccess {
    pub resource: ResourceId,
    pub mode: AccessMode,
}

impl ResourceAccess {
    /// Creates one access declaration.
    #[must_use]
    pub fn new(resource: impl Into<ResourceId>, mode: AccessMode) -> Self {
        Self {
            resource: resource.into(),
            mode,
        }
    }
}

/// Task declaration consumed by [`TaskGraphBuilder`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskSpec {
    pub id: TaskId,
    pub label: String,
    pub accesses: Vec<ResourceAccess>,
    /// Tasks that must complete before this task becomes ready.
    pub depends_on: Vec<TaskId>,
}

impl TaskSpec {
    /// Creates an empty task declaration.
    #[must_use]
    pub fn new(id: TaskId, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            accesses: Vec::new(),
            depends_on: Vec::new(),
        }
    }

    /// Adds one resource access in declaration order.
    #[must_use]
    pub fn access(mut self, resource: impl Into<ResourceId>, mode: AccessMode) -> Self {
        self.accesses.push(ResourceAccess::new(resource, mode));
        self
    }

    /// Adds one explicit dependency.
    #[must_use]
    pub fn depends_on(mut self, task: TaskId) -> Self {
        self.depends_on.push(task);
        self
    }
}

/// Why one graph edge exists.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum DependencyKind {
    /// Derived from conflicting accesses to one resource.
    DataConflict,
    /// Declared explicitly by the task author/compiler.
    Explicit,
}

/// One dependency edge with deterministic reason metadata.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencyEdge {
    pub from: TaskId,
    pub to: TaskId,
    pub kinds: BTreeSet<DependencyKind>,
}

/// Validated immutable dependency graph.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencyGraph {
    tasks: BTreeMap<TaskId, TaskSpec>,
    edges: BTreeMap<TaskId, BTreeMap<TaskId, BTreeSet<DependencyKind>>>,
}

impl DependencyGraph {
    /// Returns the number of tasks.
    #[must_use]
    pub fn task_count(&self) -> usize {
        self.tasks.len()
    }

    /// Returns one task declaration.
    #[must_use]
    pub fn task(&self, id: TaskId) -> Option<&TaskSpec> {
        self.tasks.get(&id)
    }

    /// Returns all task identities in stable order.
    #[must_use]
    pub fn task_ids(&self) -> BTreeSet<TaskId> {
        self.tasks.keys().copied().collect()
    }

    /// Returns all edges in stable `(from, to)` order.
    #[must_use]
    pub fn edges(&self) -> Vec<DependencyEdge> {
        self.edges
            .iter()
            .flat_map(|(from, targets)| {
                targets.iter().map(|(to, kinds)| DependencyEdge {
                    from: *from,
                    to: *to,
                    kinds: kinds.clone(),
                })
            })
            .collect()
    }

    /// Returns tasks that directly precede `task`.
    #[must_use]
    pub fn dependencies_of(&self, task: TaskId) -> BTreeSet<TaskId> {
        self.edges
            .iter()
            .filter_map(|(from, targets)| targets.contains_key(&task).then_some(*from))
            .collect()
    }

    /// Returns tasks that directly follow `task`.
    #[must_use]
    pub fn dependents_of(&self, task: TaskId) -> BTreeSet<TaskId> {
        self.edges
            .get(&task)
            .map(|targets| targets.keys().copied().collect())
            .unwrap_or_default()
    }

    /// Returns a deterministic topological order or a cycle diagnostic.
    pub fn topological_order(&self) -> Result<Vec<TaskId>, GraphError> {
        let mut indegree: BTreeMap<_, _> = self
            .tasks
            .keys()
            .map(|task| (*task, self.dependencies_of(*task).len()))
            .collect();
        let mut ready: BTreeSet<_> = indegree
            .iter()
            .filter_map(|(task, degree)| (*degree == 0).then_some(*task))
            .collect();
        let mut order = Vec::with_capacity(self.tasks.len());
        while let Some(task) = ready.pop_first() {
            order.push(task);
            for dependent in self.dependents_of(task) {
                let degree = indegree
                    .get_mut(&dependent)
                    .expect("edge target is a known task");
                *degree -= 1;
                if *degree == 0 {
                    ready.insert(dependent);
                }
            }
        }
        if order.len() != self.tasks.len() {
            let remaining = indegree
                .into_iter()
                .filter_map(|(task, degree)| (degree != 0).then_some(task))
                .collect();
            return Err(GraphError::Cycle { tasks: remaining });
        }
        Ok(order)
    }

    /// Returns all tasks ready after the supplied completed set.
    #[must_use]
    pub fn ready_tasks(&self, completed: &BTreeSet<TaskId>) -> BTreeSet<TaskId> {
        self.tasks
            .keys()
            .filter(|task| {
                !completed.contains(task)
                    && self
                        .dependencies_of(**task)
                        .iter()
                        .all(|dependency| completed.contains(dependency))
            })
            .copied()
            .collect()
    }
}

/// Errors found before a scheduler may consume a graph.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphError {
    DuplicateTask(TaskId),
    EmptyLabel(TaskId),
    EmptyResource(TaskId),
    DuplicateAccess { task: TaskId, resource: ResourceId },
    UnknownDependency { task: TaskId, dependency: TaskId },
    Cycle { tasks: Vec<TaskId> },
}

impl fmt::Display for GraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateTask(task) => write!(formatter, "duplicate task {}", task.value()),
            Self::EmptyLabel(task) => write!(formatter, "task {} has an empty label", task.value()),
            Self::EmptyResource(task) => {
                write!(formatter, "task {} has an empty resource", task.value())
            }
            Self::DuplicateAccess { task, resource } => write!(
                formatter,
                "task {} declares resource `{}` more than once",
                task.value(),
                resource.as_str()
            ),
            Self::UnknownDependency { task, dependency } => write!(
                formatter,
                "task {} depends on unknown task {}",
                task.value(),
                dependency.value()
            ),
            Self::Cycle { tasks } => write!(formatter, "dependency cycle through tasks {tasks:?}"),
        }
    }
}

impl Error for GraphError {}

/// One owned unit of work submitted to [`WorkerPool::execute`].
pub type TaskJob = Box<dyn FnOnce() + Send + 'static>;

/// One task whose body can cooperatively observe its scope cancellation token.
pub type ScopedTaskJob = Box<dyn FnOnce(CancellationToken) + Send + 'static>;

/// Cheap cloneable cancellation signal shared by one task scope.
#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Creates a non-cancelled token.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Requests cooperative cancellation for all holders of this token.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Returns whether cancellation was requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

/// Error returned by a scoped execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScopeError {
    Cancelled,
    Pool(PoolError),
}

impl fmt::Display for ScopeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => formatter.write_str("task scope was cooperatively cancelled"),
            Self::Pool(error) => error.fmt(formatter),
        }
    }
}

impl Error for ScopeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Pool(error) => Some(error),
            Self::Cancelled => None,
        }
    }
}

impl From<PoolError> for ScopeError {
    fn from(error: PoolError) -> Self {
        Self::Pool(error)
    }
}

/// Lexical owner for cooperative task cancellation and join.
#[derive(Clone, Debug, Default)]
pub struct TaskScope {
    token: CancellationToken,
}

impl TaskScope {
    /// Creates an active task scope.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a token that can be passed to child task bodies.
    #[must_use]
    pub fn token(&self) -> CancellationToken {
        self.token.clone()
    }

    /// Requests cancellation; running tasks must poll their token themselves.
    pub fn cancel(&self) {
        self.token.cancel();
    }

    /// Returns whether this scope has been cancelled.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    /// Runs scoped jobs on a work-stealing pool and joins every worker.
    pub fn run(
        &self,
        pool: WorkStealingPool,
        graph: &DependencyGraph,
        jobs: BTreeMap<TaskId, ScopedTaskJob>,
    ) -> Result<ExecutionReport, ScopeError> {
        let jobs = wrap_scoped_jobs(self.token(), jobs);
        let report = pool.execute(graph, jobs)?;
        if self.is_cancelled() {
            return Err(ScopeError::Cancelled);
        }
        Ok(report)
    }

    /// Runs scoped jobs in deterministic caller-thread order and joins the scope.
    pub fn run_deterministic(
        &self,
        graph: &DependencyGraph,
        jobs: BTreeMap<TaskId, ScopedTaskJob>,
    ) -> Result<ExecutionReport, ScopeError> {
        let jobs = wrap_scoped_jobs(self.token(), jobs);
        let report = DeterministicScheduler::new().execute(graph, jobs)?;
        if self.is_cancelled() {
            return Err(ScopeError::Cancelled);
        }
        Ok(report)
    }
}

fn wrap_scoped_jobs(
    token: CancellationToken,
    jobs: BTreeMap<TaskId, ScopedTaskJob>,
) -> BTreeMap<TaskId, TaskJob> {
    jobs.into_iter()
        .map(|(task, job)| {
            let token = token.clone();
            let wrapped: TaskJob = Box::new(move || {
                if !token.is_cancelled() {
                    job(token);
                }
            });
            (task, wrapped)
        })
        .collect()
}

/// Failure reported by a worker before the scope could complete.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TaskFailure {
    /// The closure panicked; the panic never unwinds across the scheduler API.
    Panicked,
}

/// Scheduler execution failure shared by the fixed and stealing pools.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PoolError {
    InvalidWorkerCount,
    JobSetMismatch {
        missing: BTreeSet<TaskId>,
        extra: BTreeSet<TaskId>,
    },
    TaskFailed {
        task: TaskId,
        failure: TaskFailure,
    },
    WorkerChannelClosed,
    WorkerPanicked,
}

impl fmt::Display for PoolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidWorkerCount => formatter.write_str("worker count must be positive"),
            Self::JobSetMismatch { missing, extra } => {
                write!(
                    formatter,
                    "job set mismatch: missing {missing:?}, extra {extra:?}"
                )
            }
            Self::TaskFailed { task, failure } => {
                write!(formatter, "task {} failed: {failure:?}", task.value())
            }
            Self::WorkerChannelClosed => formatter.write_str("worker channel closed unexpectedly"),
            Self::WorkerPanicked => formatter.write_str("worker thread panicked during join"),
        }
    }
}

impl Error for PoolError {}

/// Completion evidence returned by one fixed-pool execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionReport {
    /// Task IDs in observed completion order. Dependency order is guaranteed;
    /// independent tasks may appear in either order.
    pub completed_order: Vec<TaskId>,
    /// Number of worker threads used for this execution scope.
    pub worker_count: usize,
}

/// Chunking policy for [`WorkStealingPool::parallel_for`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ParallelForConfig {
    chunk_size: usize,
}

impl ParallelForConfig {
    /// Creates a chunk policy with a positive number of iterations per task.
    pub fn new(chunk_size: usize) -> Result<Self, ParallelForError> {
        if chunk_size == 0 {
            return Err(ParallelForError::InvalidChunkSize);
        }
        Ok(Self { chunk_size })
    }

    /// Returns the configured number of iterations per chunk.
    #[must_use]
    pub const fn chunk_size(self) -> usize {
        self.chunk_size
    }
}

/// Errors raised while constructing or executing a parallel-for batch.
#[derive(Debug, Eq, PartialEq)]
pub enum ParallelForError {
    InvalidChunkSize,
    TooManyChunks,
    Pool(PoolError),
}

impl fmt::Display for ParallelForError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidChunkSize => {
                formatter.write_str("parallel-for chunk size must be positive")
            }
            Self::TooManyChunks => {
                formatter.write_str("parallel-for has more chunks than TaskId can represent")
            }
            Self::Pool(error) => error.fmt(formatter),
        }
    }
}

impl Error for ParallelForError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Pool(error) => Some(error),
            Self::InvalidChunkSize | Self::TooManyChunks => None,
        }
    }
}

impl From<PoolError> for ParallelForError {
    fn from(error: PoolError) -> Self {
        Self::Pool(error)
    }
}

/// Completion evidence returned by one parallel-for execution.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParallelForReport {
    /// Number of iterations requested by the caller.
    pub iterations: usize,
    /// Number of generated independent chunks.
    pub chunks: usize,
    /// Completion order of chunk task IDs; independent chunks may vary.
    pub completed_chunks: Vec<TaskId>,
    /// Number of worker threads used for this execution scope.
    pub worker_count: usize,
}

/// Errors raised while constructing or executing a parallel reduction.
#[derive(Debug, Eq, PartialEq)]
pub enum ParallelReduceError {
    InvalidChunkSize,
    TooManyChunks,
    MissingChunk(usize),
    Pool(PoolError),
}

impl fmt::Display for ParallelReduceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidChunkSize => {
                formatter.write_str("parallel-reduce chunk size must be positive")
            }
            Self::TooManyChunks => {
                formatter.write_str("parallel-reduce has more chunks than TaskId can represent")
            }
            Self::MissingChunk(chunk) => {
                write!(formatter, "parallel-reduce chunk {chunk} did not complete")
            }
            Self::Pool(error) => error.fmt(formatter),
        }
    }
}

impl Error for ParallelReduceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Pool(error) => Some(error),
            Self::InvalidChunkSize | Self::TooManyChunks | Self::MissingChunk(_) => None,
        }
    }
}

impl From<PoolError> for ParallelReduceError {
    fn from(error: PoolError) -> Self {
        Self::Pool(error)
    }
}

/// Completion evidence and value returned by one parallel reduction.
#[derive(Debug, Eq, PartialEq)]
pub struct ParallelReduceReport<T> {
    /// Reduced value after stable chunk-order combination.
    pub value: T,
    /// Number of iterations requested by the caller.
    pub iterations: usize,
    /// Number of generated independent chunks.
    pub chunks: usize,
    /// Completion order of chunk task IDs; independent chunks may vary.
    pub completed_chunks: Vec<TaskId>,
    /// Number of worker threads used for this execution scope.
    pub worker_count: usize,
}

enum WorkMessage {
    Run(TaskId, TaskJob),
    Shutdown,
}

struct StealQueueState {
    queues: Vec<Mutex<VecDeque<WorkMessage>>>,
    shutdown: AtomicBool,
    wake: Condvar,
    wake_lock: Mutex<()>,
}

impl StealQueueState {
    fn new(worker_count: usize) -> Self {
        Self {
            queues: (0..worker_count)
                .map(|_| Mutex::new(VecDeque::new()))
                .collect(),
            shutdown: AtomicBool::new(false),
            wake: Condvar::new(),
            wake_lock: Mutex::new(()),
        }
    }

    fn enqueue(&self, worker: usize, message: WorkMessage) {
        let _wake_guard = self
            .wake_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut queue = self.queues[worker]
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        queue.push_back(message);
        drop(queue);
        self.wake.notify_all();
    }

    fn take(&self, worker: usize) -> Option<WorkMessage> {
        if let Some(message) = self.queues[worker]
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop_back()
        {
            return Some(message);
        }
        for offset in 1..self.queues.len() {
            let victim = (worker + offset) % self.queues.len();
            if let Some(message) = self.queues[victim]
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .pop_front()
            {
                return Some(message);
            }
        }
        None
    }

    fn has_pending(&self) -> bool {
        self.queues.iter().any(|queue| {
            !queue
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty()
        })
    }

    fn wait_for_work(&self) {
        let mut guard = self
            .wake_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while !self.shutdown.load(Ordering::Acquire) && !self.has_pending() {
            guard = self
                .wake
                .wait(guard)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    fn request_shutdown(&self) {
        let _wake_guard = self
            .wake_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.shutdown.store(true, Ordering::Release);
        self.wake.notify_all();
    }
}

/// Fixed worker pool that consumes a validated dependency graph.
///
/// The MVP creates one bounded worker set per `execute` scope and joins every
/// thread before returning. It intentionally has no detached tasks, stealing,
/// cancellation or hidden global executor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkerPool {
    worker_count: usize,
}

impl WorkerPool {
    /// Creates a pool with a fixed positive worker count.
    pub fn new(worker_count: usize) -> Result<Self, PoolError> {
        if worker_count == 0 {
            return Err(PoolError::InvalidWorkerCount);
        }
        Ok(Self { worker_count })
    }

    /// Returns the configured worker count.
    #[must_use]
    pub const fn worker_count(self) -> usize {
        self.worker_count
    }

    /// Executes every graph task exactly once and joins all workers.
    pub fn execute(
        self,
        graph: &DependencyGraph,
        mut jobs: BTreeMap<TaskId, TaskJob>,
    ) -> Result<ExecutionReport, PoolError> {
        let expected = graph.task_ids();
        let actual: BTreeSet<_> = jobs.keys().copied().collect();
        if expected != actual {
            let missing = expected.difference(&actual).copied().collect();
            let extra = actual.difference(&expected).copied().collect();
            return Err(PoolError::JobSetMismatch { missing, extra });
        }

        let (work_sender, work_receiver) = mpsc::channel();
        let (completion_sender, completion_receiver) = mpsc::channel();
        let shared_receiver = Arc::new(Mutex::new(work_receiver));
        let workers = spawn_workers(self.worker_count, shared_receiver, completion_sender);
        let mut submitted = BTreeSet::new();
        let mut completed = BTreeSet::new();
        let mut running = 0usize;
        let mut completed_order = Vec::with_capacity(expected.len());
        let execution_result = loop {
            let ready = graph.ready_tasks(&completed);
            let mut dispatch_failed = false;
            for task in ready {
                if running >= self.worker_count {
                    break;
                }
                if submitted.contains(&task) {
                    continue;
                }
                let job = jobs.remove(&task).expect("validated graph/job set");
                if work_sender.send(WorkMessage::Run(task, job)).is_err() {
                    dispatch_failed = true;
                    break;
                }
                submitted.insert(task);
                running += 1;
            }
            if dispatch_failed {
                break Err(PoolError::WorkerChannelClosed);
            }
            if completed.len() == expected.len() {
                break Ok(ExecutionReport {
                    completed_order,
                    worker_count: self.worker_count,
                });
            }
            if running == 0 {
                break Err(PoolError::WorkerChannelClosed);
            }
            let (task, result) = match completion_receiver.recv() {
                Ok(completion) => completion,
                Err(_) => break Err(PoolError::WorkerChannelClosed),
            };
            running -= 1;
            if let Err(failure) = result {
                break Err(PoolError::TaskFailed { task, failure });
            }
            completed.insert(task);
            completed_order.push(task);
        };

        for _ in 0..self.worker_count {
            let _ = work_sender.send(WorkMessage::Shutdown);
        }
        drop(work_sender);
        join_workers(workers)?;
        execution_result
    }
}

/// Deterministic scheduler mode for replay, tests and diagnostics.
///
/// The scheduler executes the validated graph in stable topological order on
/// the caller thread. It intentionally reports one logical worker lane and
/// creates no OS worker thread, so independent-task completion cannot vary
/// with thread timing.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DeterministicScheduler;

impl DeterministicScheduler {
    /// Creates a deterministic scheduler.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Executes every graph task in stable topological order.
    pub fn execute(
        self,
        graph: &DependencyGraph,
        mut jobs: BTreeMap<TaskId, TaskJob>,
    ) -> Result<ExecutionReport, PoolError> {
        let expected = graph.task_ids();
        let actual: BTreeSet<_> = jobs.keys().copied().collect();
        if expected != actual {
            let missing = expected.difference(&actual).copied().collect();
            let extra = actual.difference(&expected).copied().collect();
            return Err(PoolError::JobSetMismatch { missing, extra });
        }
        let mut completed_order = Vec::with_capacity(expected.len());
        for task in graph
            .topological_order()
            .expect("scheduler input graph is validated and acyclic")
        {
            let job = jobs.remove(&task).expect("validated graph/job set");
            if catch_unwind(AssertUnwindSafe(job)).is_err() {
                return Err(PoolError::TaskFailed {
                    task,
                    failure: TaskFailure::Panicked,
                });
            }
            completed_order.push(task);
        }
        Ok(ExecutionReport {
            completed_order,
            worker_count: 1,
        })
    }
}

/// Fixed-size work-stealing pool that consumes a validated dependency graph.
///
/// Each worker owns a deque. It executes its own newest job first and steals
/// the oldest job from another worker when its local deque is empty. The pool
/// is scoped to one call, has no detached workers and joins every thread before
/// returning. A failed task requests shutdown; queued but not-yet-started jobs
/// are then dropped and the original [`PoolError::TaskFailed`] is returned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkStealingPool {
    worker_count: usize,
}

impl WorkStealingPool {
    /// Creates a stealing pool with a fixed positive worker count.
    pub fn new(worker_count: usize) -> Result<Self, PoolError> {
        if worker_count == 0 {
            return Err(PoolError::InvalidWorkerCount);
        }
        Ok(Self { worker_count })
    }

    /// Returns the configured worker count.
    #[must_use]
    pub const fn worker_count(self) -> usize {
        self.worker_count
    }

    /// Runs a side-effecting body over `0..iterations` in independent chunks.
    ///
    /// The body is shared immutably between chunk jobs and must therefore be
    /// `Send + Sync`. A body panic is converted to [`ParallelForError::Pool`]
    /// and causes the scope to shut down; queued chunks are dropped.
    pub fn parallel_for<F>(
        self,
        iterations: usize,
        config: ParallelForConfig,
        body: F,
    ) -> Result<ParallelForReport, ParallelForError>
    where
        F: Fn(usize) + Send + Sync + 'static,
    {
        let chunk_size = config.chunk_size();
        let chunks = iterations / chunk_size + usize::from(!iterations.is_multiple_of(chunk_size));
        if chunks > u32::MAX as usize {
            return Err(ParallelForError::TooManyChunks);
        }
        let mut builder = TaskGraphBuilder::default();
        let body = Arc::new(body);
        let mut jobs = BTreeMap::new();
        for chunk in 0..chunks {
            let start = chunk * chunk_size;
            let end = (start + chunk_size).min(iterations);
            let task_id = TaskId::new(chunk as u32);
            builder
                .add_task(TaskSpec::new(task_id, format!("parallel_for[{chunk}]")))
                .expect("generated parallel-for task is valid");
            let body = Arc::clone(&body);
            jobs.insert(
                task_id,
                Box::new(move || {
                    for index in start..end {
                        body(index);
                    }
                }) as TaskJob,
            );
        }
        let graph = builder
            .build()
            .expect("generated parallel-for graph is acyclic");
        let report = self.execute(&graph, jobs)?;
        Ok(ParallelForReport {
            iterations,
            chunks,
            completed_chunks: report.completed_order,
            worker_count: report.worker_count,
        })
    }

    /// Maps and reduces `0..iterations` with deterministic final combination.
    ///
    /// Each chunk starts with a clone of `initial` and reduces its own mapped
    /// values sequentially. Once all chunks complete, their partial values are
    /// combined in ascending chunk ID order, so the final reduction order does
    /// not depend on worker completion timing.
    pub fn parallel_reduce<T, Map, Reduce>(
        self,
        iterations: usize,
        config: ParallelForConfig,
        initial: T,
        map: Map,
        reduce: Reduce,
    ) -> Result<ParallelReduceReport<T>, ParallelReduceError>
    where
        T: Clone + Send + 'static,
        Map: Fn(usize) -> T + Send + Sync + 'static,
        Reduce: Fn(T, T) -> T + Send + Sync + 'static,
    {
        let chunk_size = config.chunk_size();
        let chunks = iterations / chunk_size + usize::from(!iterations.is_multiple_of(chunk_size));
        if chunks > u32::MAX as usize {
            return Err(ParallelReduceError::TooManyChunks);
        }
        let mut builder = TaskGraphBuilder::default();
        let map = Arc::new(map);
        let reduce = Arc::new(reduce);
        let partials = Arc::new(Mutex::new(
            (0..chunks).map(|_| None).collect::<Vec<Option<T>>>(),
        ));
        let mut jobs = BTreeMap::new();
        for chunk in 0..chunks {
            let start = chunk * chunk_size;
            let end = (start + chunk_size).min(iterations);
            let task_id = TaskId::new(chunk as u32);
            builder
                .add_task(TaskSpec::new(task_id, format!("parallel_reduce[{chunk}]")))
                .expect("generated parallel-reduce task is valid");
            let map = Arc::clone(&map);
            let reduce = Arc::clone(&reduce);
            let partials = Arc::clone(&partials);
            let initial = initial.clone();
            jobs.insert(
                task_id,
                Box::new(move || {
                    let mut value = initial;
                    for index in start..end {
                        value = reduce(value, map(index));
                    }
                    partials
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())[chunk] = Some(value);
                }) as TaskJob,
            );
        }
        let graph = builder
            .build()
            .expect("generated parallel-reduce graph is acyclic");
        let report = self.execute(&graph, jobs)?;
        let mut value = initial;
        let mut partials = partials
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for chunk in 0..chunks {
            let partial = partials[chunk]
                .take()
                .ok_or(ParallelReduceError::MissingChunk(chunk))?;
            value = reduce(value, partial);
        }
        Ok(ParallelReduceReport {
            value,
            iterations,
            chunks,
            completed_chunks: report.completed_order,
            worker_count: report.worker_count,
        })
    }

    /// Executes every graph task exactly once and joins all workers.
    pub fn execute(
        self,
        graph: &DependencyGraph,
        mut jobs: BTreeMap<TaskId, TaskJob>,
    ) -> Result<ExecutionReport, PoolError> {
        let expected = graph.task_ids();
        let actual: BTreeSet<_> = jobs.keys().copied().collect();
        if expected != actual {
            let missing = expected.difference(&actual).copied().collect();
            let extra = actual.difference(&expected).copied().collect();
            return Err(PoolError::JobSetMismatch { missing, extra });
        }

        let state = Arc::new(StealQueueState::new(self.worker_count));
        let (completion_sender, completion_receiver) = mpsc::channel();
        let workers =
            spawn_stealing_workers(self.worker_count, Arc::clone(&state), completion_sender);
        let mut submitted = BTreeSet::new();
        let mut completed = BTreeSet::new();
        let mut running = 0usize;
        let mut next_worker = 0usize;
        let mut completed_order = Vec::with_capacity(expected.len());
        let execution_result = loop {
            for task in graph.ready_tasks(&completed) {
                if submitted.contains(&task) {
                    continue;
                }
                let job = jobs.remove(&task).expect("validated graph/job set");
                state.enqueue(next_worker, WorkMessage::Run(task, job));
                next_worker = (next_worker + 1) % self.worker_count;
                submitted.insert(task);
                running += 1;
            }
            if completed.len() == expected.len() {
                break Ok(ExecutionReport {
                    completed_order,
                    worker_count: self.worker_count,
                });
            }
            if running == 0 {
                break Err(PoolError::WorkerChannelClosed);
            }
            let (task, result) = match completion_receiver.recv() {
                Ok(completion) => completion,
                Err(_) => break Err(PoolError::WorkerChannelClosed),
            };
            running -= 1;
            if let Err(failure) = result {
                break Err(PoolError::TaskFailed { task, failure });
            }
            completed.insert(task);
            completed_order.push(task);
        };

        state.request_shutdown();
        drop(completion_receiver);
        join_workers(workers)?;
        execution_result
    }
}

fn spawn_workers(
    count: usize,
    receiver: Arc<Mutex<Receiver<WorkMessage>>>,
    completion_sender: Sender<(TaskId, Result<(), TaskFailure>)>,
) -> Vec<JoinHandle<()>> {
    (0..count)
        .map(|_| {
            let receiver = Arc::clone(&receiver);
            let completion_sender = completion_sender.clone();
            thread::spawn(move || {
                loop {
                    let message = receiver.lock().ok().and_then(|guard| guard.recv().ok());
                    match message {
                        Some(WorkMessage::Run(task, job)) => {
                            let result = catch_unwind(AssertUnwindSafe(job))
                                .map(|_| ())
                                .map_err(|_| TaskFailure::Panicked);
                            if completion_sender.send((task, result)).is_err() {
                                break;
                            }
                        }
                        Some(WorkMessage::Shutdown) | None => break,
                    }
                }
            })
        })
        .collect()
}

fn join_workers(workers: Vec<JoinHandle<()>>) -> Result<(), PoolError> {
    let mut panicked = false;
    for worker in workers {
        if worker.join().is_err() {
            panicked = true;
        }
    }
    if panicked {
        return Err(PoolError::WorkerPanicked);
    }
    Ok(())
}

fn spawn_stealing_workers(
    count: usize,
    state: Arc<StealQueueState>,
    completion_sender: Sender<(TaskId, Result<(), TaskFailure>)>,
) -> Vec<JoinHandle<()>> {
    (0..count)
        .map(|worker| {
            let state = Arc::clone(&state);
            let completion_sender = completion_sender.clone();
            thread::spawn(move || {
                loop {
                    let Some(message) = state.take(worker) else {
                        if state.shutdown.load(Ordering::Acquire) {
                            break;
                        }
                        state.wait_for_work();
                        continue;
                    };
                    match message {
                        WorkMessage::Run(task, job) => {
                            let result = catch_unwind(AssertUnwindSafe(job))
                                .map(|_| ())
                                .map_err(|_| TaskFailure::Panicked);
                            if completion_sender.send((task, result)).is_err() {
                                break;
                            }
                        }
                        WorkMessage::Shutdown => break,
                    }
                }
            })
        })
        .collect()
}

/// Builds a graph with stable ordering and conservative conflict edges.
#[derive(Clone, Debug, Default)]
pub struct TaskGraphBuilder {
    tasks: BTreeMap<TaskId, TaskSpec>,
}

impl TaskGraphBuilder {
    /// Adds one validated task declaration.
    pub fn add_task(&mut self, task: TaskSpec) -> Result<(), GraphError> {
        if task.label.trim().is_empty() {
            return Err(GraphError::EmptyLabel(task.id));
        }
        if self.tasks.contains_key(&task.id) {
            return Err(GraphError::DuplicateTask(task.id));
        }
        let mut resources = BTreeSet::new();
        for access in &task.accesses {
            if access.resource.as_str().trim().is_empty() {
                return Err(GraphError::EmptyResource(task.id));
            }
            if !resources.insert(access.resource.clone()) {
                return Err(GraphError::DuplicateAccess {
                    task: task.id,
                    resource: access.resource.clone(),
                });
            }
        }
        self.tasks.insert(task.id, task);
        Ok(())
    }

    /// Validates explicit dependencies, derives conflict edges and detects cycles.
    pub fn build(self) -> Result<DependencyGraph, GraphError> {
        for task in self.tasks.values() {
            for dependency in &task.depends_on {
                if !self.tasks.contains_key(dependency) {
                    return Err(GraphError::UnknownDependency {
                        task: task.id,
                        dependency: *dependency,
                    });
                }
            }
        }
        let mut edges: BTreeMap<_, BTreeMap<_, BTreeSet<_>>> = BTreeMap::new();
        for task in self.tasks.values() {
            for dependency in &task.depends_on {
                edges
                    .entry(*dependency)
                    .or_default()
                    .entry(task.id)
                    .or_default()
                    .insert(DependencyKind::Explicit);
            }
        }
        let task_list: Vec<_> = self.tasks.values().collect();
        for (left_index, left) in task_list.iter().enumerate() {
            for right in task_list.iter().skip(left_index + 1) {
                if accesses_conflict(&left.accesses, &right.accesses) {
                    edges
                        .entry(left.id)
                        .or_default()
                        .entry(right.id)
                        .or_default()
                        .insert(DependencyKind::DataConflict);
                }
            }
        }
        let graph = DependencyGraph {
            tasks: self.tasks,
            edges,
        };
        graph.topological_order()?;
        Ok(graph)
    }
}

fn accesses_conflict(left: &[ResourceAccess], right: &[ResourceAccess]) -> bool {
    left.iter().any(|left_access| {
        right.iter().any(|right_access| {
            left_access.resource == right_access.resource
                && left_access.mode.conflicts(right_access.mode)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: u32, label: &str) -> TaskSpec {
        TaskSpec::new(TaskId::new(id), label)
    }

    #[test]
    fn read_read_accesses_can_overlap() {
        let mut builder = TaskGraphBuilder::default();
        builder
            .add_task(task(1, "read-a").access("positions", AccessMode::Read))
            .unwrap();
        builder
            .add_task(task(2, "read-b").access("positions", AccessMode::Read))
            .unwrap();
        let graph = builder.build().unwrap();
        assert!(graph.edges().is_empty());
        assert_eq!(
            graph.topological_order().unwrap(),
            vec![TaskId::new(1), TaskId::new(2)]
        );
    }

    #[test]
    fn write_conflicts_create_stable_forward_edge() {
        let mut builder = TaskGraphBuilder::default();
        builder
            .add_task(task(2, "write").access("positions", AccessMode::Write))
            .unwrap();
        builder
            .add_task(task(7, "read").access("positions", AccessMode::Read))
            .unwrap();
        let graph = builder.build().unwrap();
        assert_eq!(
            graph.dependencies_of(TaskId::new(7)),
            [TaskId::new(2)].into()
        );
        assert_eq!(
            graph.edges()[0].kinds,
            [DependencyKind::DataConflict].into()
        );
    }

    #[test]
    fn explicit_dependency_is_preserved_and_ready_set_is_deterministic() {
        let mut builder = TaskGraphBuilder::default();
        builder.add_task(task(1, "load")).unwrap();
        builder
            .add_task(task(3, "compute").depends_on(TaskId::new(1)))
            .unwrap();
        builder
            .add_task(task(2, "independent").access("other", AccessMode::Write))
            .unwrap();
        let graph = builder.build().unwrap();
        assert_eq!(
            graph.ready_tasks(&BTreeSet::new()),
            [TaskId::new(1), TaskId::new(2)].into()
        );
        assert_eq!(
            graph.ready_tasks(&[TaskId::new(1)].into()),
            [TaskId::new(2), TaskId::new(3)].into()
        );
    }

    #[test]
    fn explicit_cycle_is_rejected() {
        let mut builder = TaskGraphBuilder::default();
        builder
            .add_task(task(1, "a").depends_on(TaskId::new(2)))
            .unwrap();
        builder
            .add_task(task(2, "b").depends_on(TaskId::new(1)))
            .unwrap();
        assert!(matches!(
            builder.build(),
            Err(GraphError::Cycle { tasks }) if tasks == vec![TaskId::new(1), TaskId::new(2)]
        ));
    }

    #[test]
    fn malformed_task_accesses_are_rejected() {
        let mut builder = TaskGraphBuilder::default();
        assert_eq!(
            builder.add_task(task(1, " ")),
            Err(GraphError::EmptyLabel(TaskId::new(1)))
        );
        assert_eq!(
            builder.add_task(
                task(2, "dup")
                    .access("x", AccessMode::Read)
                    .access("x", AccessMode::Write)
            ),
            Err(GraphError::DuplicateAccess {
                task: TaskId::new(2),
                resource: ResourceId::new("x")
            })
        );
    }

    #[test]
    fn fixed_pool_executes_dependencies_and_joins_workers() {
        use std::sync::{Arc, Mutex};

        let mut builder = TaskGraphBuilder::default();
        builder.add_task(task(1, "load")).unwrap();
        builder
            .add_task(task(2, "write").depends_on(TaskId::new(1)))
            .unwrap();
        builder
            .add_task(task(3, "read").depends_on(TaskId::new(2)))
            .unwrap();
        let graph = builder.build().unwrap();
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut jobs = BTreeMap::new();
        for id in 1..=3 {
            let order = Arc::clone(&order);
            jobs.insert(
                TaskId::new(id),
                Box::new(move || order.lock().unwrap().push(id)) as TaskJob,
            );
        }
        let report = WorkerPool::new(2).unwrap().execute(&graph, jobs).unwrap();
        assert_eq!(
            report.completed_order,
            vec![TaskId::new(1), TaskId::new(2), TaskId::new(3)]
        );
        assert_eq!(*order.lock().unwrap(), vec![1, 2, 3]);
    }

    #[test]
    fn fixed_pool_converts_task_panic_to_error_and_joins() {
        let mut builder = TaskGraphBuilder::default();
        builder.add_task(task(1, "panic")).unwrap();
        let graph = builder.build().unwrap();
        let mut jobs = BTreeMap::new();
        jobs.insert(
            TaskId::new(1),
            Box::new(|| -> () { panic!("expected") }) as TaskJob,
        );
        assert_eq!(
            WorkerPool::new(1).unwrap().execute(&graph, jobs),
            Err(PoolError::TaskFailed {
                task: TaskId::new(1),
                failure: TaskFailure::Panicked
            })
        );
    }

    #[test]
    fn deterministic_scheduler_runs_independent_tasks_in_task_id_order() {
        use std::sync::{Arc, Mutex};

        let mut builder = TaskGraphBuilder::default();
        builder.add_task(task(3, "third")).unwrap();
        builder.add_task(task(1, "first")).unwrap();
        builder.add_task(task(2, "second")).unwrap();
        let graph = builder.build().unwrap();
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut jobs = BTreeMap::new();
        for id in [1, 2, 3] {
            let order = Arc::clone(&order);
            jobs.insert(
                TaskId::new(id),
                Box::new(move || order.lock().unwrap().push(id)) as TaskJob,
            );
        }
        let report = DeterministicScheduler::new().execute(&graph, jobs).unwrap();
        assert_eq!(
            report.completed_order,
            vec![TaskId::new(1), TaskId::new(2), TaskId::new(3)]
        );
        assert_eq!(*order.lock().unwrap(), vec![1, 2, 3]);
        assert_eq!(report.worker_count, 1);
    }

    #[test]
    fn stealing_pool_preserves_dependencies_with_independent_work() {
        use std::sync::{Arc, Mutex};

        let mut builder = TaskGraphBuilder::default();
        builder.add_task(task(1, "left")).unwrap();
        builder.add_task(task(2, "right")).unwrap();
        builder
            .add_task(task(3, "left-child").depends_on(TaskId::new(1)))
            .unwrap();
        builder
            .add_task(task(4, "right-child").depends_on(TaskId::new(2)))
            .unwrap();
        let graph = builder.build().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut jobs = BTreeMap::new();
        for id in 1..=4 {
            let seen = Arc::clone(&seen);
            jobs.insert(
                TaskId::new(id),
                Box::new(move || seen.lock().unwrap().push(id)) as TaskJob,
            );
        }
        let report = WorkStealingPool::new(2)
            .unwrap()
            .execute(&graph, jobs)
            .unwrap();
        assert_eq!(report.completed_order.len(), 4);
        let position = |id| {
            report
                .completed_order
                .iter()
                .position(|completed| *completed == TaskId::new(id))
                .unwrap()
        };
        assert!(position(1) < position(3));
        assert!(position(2) < position(4));
        assert_eq!(seen.lock().unwrap().len(), 4);
    }

    #[test]
    fn stealing_pool_converts_task_panic_to_error_and_joins() {
        let mut builder = TaskGraphBuilder::default();
        builder.add_task(task(1, "panic")).unwrap();
        let graph = builder.build().unwrap();
        let mut jobs = BTreeMap::new();
        jobs.insert(
            TaskId::new(1),
            Box::new(|| -> () { panic!("expected") }) as TaskJob,
        );
        assert_eq!(
            WorkStealingPool::new(2).unwrap().execute(&graph, jobs),
            Err(PoolError::TaskFailed {
                task: TaskId::new(1),
                failure: TaskFailure::Panicked
            })
        );
    }

    #[test]
    fn task_scope_cancellation_skips_dependent_jobs_and_joins() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let mut builder = TaskGraphBuilder::default();
        builder.add_task(task(1, "cancel")).unwrap();
        builder
            .add_task(task(2, "skipped").depends_on(TaskId::new(1)))
            .unwrap();
        let graph = builder.build().unwrap();
        let executed = Arc::new(AtomicUsize::new(0));
        let mut jobs = BTreeMap::new();
        let first_executed = Arc::clone(&executed);
        jobs.insert(
            TaskId::new(1),
            Box::new(move |token: CancellationToken| {
                first_executed.fetch_add(1, Ordering::SeqCst);
                token.cancel();
            }) as ScopedTaskJob,
        );
        let second_executed = Arc::clone(&executed);
        jobs.insert(
            TaskId::new(2),
            Box::new(move |_token| {
                second_executed.fetch_add(1, Ordering::SeqCst);
            }) as ScopedTaskJob,
        );
        let scope = TaskScope::new();
        assert_eq!(
            scope.run(WorkStealingPool::new(2).unwrap(), &graph, jobs),
            Err(ScopeError::Cancelled)
        );
        assert_eq!(executed.load(Ordering::SeqCst), 1);
        assert!(scope.is_cancelled());
    }

    #[test]
    fn task_scope_can_be_cancelled_before_dispatch() {
        let mut builder = TaskGraphBuilder::default();
        builder.add_task(task(1, "skipped")).unwrap();
        let graph = builder.build().unwrap();
        let mut jobs = BTreeMap::new();
        jobs.insert(TaskId::new(1), Box::new(|_token| {}) as ScopedTaskJob);
        let scope = TaskScope::new();
        scope.cancel();
        assert_eq!(
            scope.run(WorkStealingPool::new(1).unwrap(), &graph, jobs),
            Err(ScopeError::Cancelled)
        );
    }

    #[test]
    fn parallel_for_batches_all_iterations() {
        use std::sync::{Arc, Mutex};

        let seen = Arc::new(Mutex::new(Vec::new()));
        let body_seen = Arc::clone(&seen);
        let config = ParallelForConfig::new(3).unwrap();
        let report = WorkStealingPool::new(2)
            .unwrap()
            .parallel_for(10, config, move |index| {
                body_seen.lock().unwrap().push(index);
            })
            .unwrap();
        assert_eq!(report.iterations, 10);
        assert_eq!(report.chunks, 4);
        assert_eq!(report.completed_chunks.len(), 4);
        let mut values = seen.lock().unwrap().clone();
        values.sort_unstable();
        assert_eq!(values, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn parallel_for_rejects_zero_chunk_and_propagates_panic() {
        assert_eq!(
            ParallelForConfig::new(0),
            Err(ParallelForError::InvalidChunkSize)
        );
        let config = ParallelForConfig::new(1).unwrap();
        let error = WorkStealingPool::new(2)
            .unwrap()
            .parallel_for(2, config, |_index| -> () { panic!("expected") })
            .unwrap_err();
        assert!(matches!(
            error,
            ParallelForError::Pool(PoolError::TaskFailed {
                failure: TaskFailure::Panicked,
                ..
            })
        ));
    }

    #[test]
    fn parallel_reduce_combines_chunks_in_stable_order() {
        let config = ParallelForConfig::new(3).unwrap();
        let report = WorkStealingPool::new(2)
            .unwrap()
            .parallel_reduce(
                10,
                config,
                0usize,
                |index| index,
                |left, right| left + right,
            )
            .unwrap();
        assert_eq!(report.value, 45);
        assert_eq!(report.iterations, 10);
        assert_eq!(report.chunks, 4);
        assert_eq!(report.completed_chunks.len(), 4);
    }

    #[test]
    fn parallel_reduce_propagates_mapper_panic() {
        let config = ParallelForConfig::new(1).unwrap();
        let error = WorkStealingPool::new(2)
            .unwrap()
            .parallel_reduce(
                2,
                config,
                0usize,
                |_index| -> usize { panic!("expected") },
                |left, right| left + right,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            ParallelReduceError::Pool(PoolError::TaskFailed {
                failure: TaskFailure::Panicked,
                ..
            })
        ));
    }
}
