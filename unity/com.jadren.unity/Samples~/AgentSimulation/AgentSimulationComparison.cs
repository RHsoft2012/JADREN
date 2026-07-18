using System;
using Unity.Collections;

namespace Jadren.Unity.Samples.AgentSimulation
{
    public readonly struct AgentSimulationComparisonResult
    {
        public AgentSimulationComparisonResult(
            int count,
            int steps,
            float deltaTime,
            double managedChecksum,
            double candidateChecksum,
            double tolerance)
        {
            Count = count;
            Steps = steps;
            DeltaTime = deltaTime;
            ManagedChecksum = managedChecksum;
            CandidateChecksum = candidateChecksum;
            AbsoluteDelta = Math.Abs(managedChecksum - candidateChecksum);
            Tolerance = tolerance;
        }

        public int Count { get; }
        public int Steps { get; }
        public float DeltaTime { get; }
        public double ManagedChecksum { get; }
        public double CandidateChecksum { get; }
        public double AbsoluteDelta { get; }
        public double Tolerance { get; }
        public bool Matches => AbsoluteDelta <= Tolerance;
    }

    /// <summary>
    /// Correctness-only harness. The candidate callback is orchestration and is
    /// not part of a performance measurement; a benchmark must schedule the
    /// Burst job directly and use this result only as a preflight gate.
    /// </summary>
    public static class AgentSimulationComparison
    {
        public static AgentSimulationComparisonResult Run(
            int count,
            int steps,
            float deltaTime,
            Allocator allocator,
            Action<NativeArray<AgentState>, float> candidate,
            double tolerance = 0.000001)
        {
            if (count < 0)
            {
                throw new ArgumentOutOfRangeException(nameof(count));
            }
            if (steps < 0)
            {
                throw new ArgumentOutOfRangeException(nameof(steps));
            }
            if (float.IsNaN(deltaTime) || float.IsInfinity(deltaTime) || deltaTime < 0.0f)
            {
                throw new ArgumentOutOfRangeException(nameof(deltaTime));
            }
            if (candidate == null)
            {
                throw new ArgumentNullException(nameof(candidate));
            }
            if (tolerance < 0.0 || double.IsNaN(tolerance) || double.IsInfinity(tolerance))
            {
                throw new ArgumentOutOfRangeException(nameof(tolerance));
            }

            using (var managed = new NativeArray<AgentState>(count, allocator))
            using (var candidateArray = new NativeArray<AgentState>(count, allocator))
            {
                AgentSimulationWorkload.Initialize(managed);
                AgentSimulationWorkload.Initialize(candidateArray);
                for (var step = 0; step < steps; step++)
                {
                    AgentSimulationWorkload.StepManaged(managed, deltaTime);
                    candidate(candidateArray, deltaTime);
                }

                return new AgentSimulationComparisonResult(
                    count,
                    steps,
                    deltaTime,
                    AgentSimulationWorkload.Checksum(managed),
                    AgentSimulationWorkload.Checksum(candidateArray),
                    tolerance);
            }
        }
    }
}
