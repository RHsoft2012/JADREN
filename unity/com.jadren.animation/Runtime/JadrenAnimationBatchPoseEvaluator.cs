using System;
using UnityEngine;

namespace Jadren.Animation
{
    /// <summary>
    /// Shared pose evaluator for a crowd that uses one rig/controller pair.
    /// Agent time, transition and output buffers remain independent, while
    /// clip snapshots and the evaluator hot path are shared. Unity object
    /// access is kept at the caller-owned applier boundary.
    /// </summary>
    public sealed class JadrenAnimationBatchPoseEvaluator : IDisposable
    {
        private readonly JadrenAnimationPoseWorker worker;
        private readonly JadrenControllerAsset controller;
        private readonly JadrenPoseBuffer[] poses;
        private readonly JadrenAnimationState[] states;
        private readonly int[] activeStates;
        private readonly int[] previousStates;
        private readonly float[] stateTimes;
        private readonly float[] previousStateTimes;
        private readonly float[] transitionWeights;
        private readonly JadrenAnimationPoseBatchRequest[] batchRequests;
        private readonly bool[] batchPrepared;
        private bool disposed;

        public int AgentCount { get { return poses.Length; } }
        public bool IsConfigured { get { return worker != null && controller != null; } }
        public bool UsesNativeSlerp { get { return worker != null && worker.UsesNativeSlerp; } }
        public bool UsesNativePoseTiles { get { return worker != null && worker.UsesNativePoseTiles; } }
        public bool UsesNativeCrowdPoseTiles { get { return worker != null && worker.UsesNativeCrowdPoseTiles; } }

        public JadrenAnimationBatchPoseEvaluator(
            JadrenRigAsset rig,
            JadrenControllerAsset controller,
            int agentCount,
            bool preferNativeSlerp = false,
            bool preferNativePoseTiles = false)
        {
            if (rig == null) throw new ArgumentNullException(nameof(rig));
            if (controller == null) throw new ArgumentNullException(nameof(controller));
            if (agentCount < 1) throw new ArgumentOutOfRangeException(nameof(agentCount));

            this.controller = controller;
            worker = new JadrenAnimationPoseWorker(
                rig,
                controller,
                preferNativeSlerp,
                preferNativePoseTiles);
            poses = new JadrenPoseBuffer[agentCount];
            states = new JadrenAnimationState[agentCount];
            activeStates = new int[agentCount];
            previousStates = new int[agentCount];
            stateTimes = new float[agentCount];
            previousStateTimes = new float[agentCount];
            transitionWeights = new float[agentCount];
            batchRequests = new JadrenAnimationPoseBatchRequest[agentCount];
            batchPrepared = new bool[agentCount];
            for (var index = 0; index < agentCount; index++)
            {
                poses[index] = new JadrenPoseBuffer();
                ResetAgent(index);
            }
        }

        public JadrenPoseBuffer GetPose(int agentIndex)
        {
            ThrowIfDisposed();
            return agentIndex >= 0 && agentIndex < poses.Length ? poses[agentIndex] : null;
        }

        public JadrenAnimationState GetState(int agentIndex)
        {
            ThrowIfDisposed();
            return agentIndex >= 0 && agentIndex < states.Length
                ? states[agentIndex]
                : JadrenAnimationState.Default;
        }

        public bool Step(
            int agentIndex,
            float deltaTime,
            float speed,
            JadrenAnimationLod lod)
        {
            ThrowIfDisposed();
            if (agentIndex < 0 || agentIndex >= poses.Length)
            {
                throw new ArgumentOutOfRangeException(nameof(agentIndex));
            }
            if (float.IsNaN(deltaTime) || float.IsInfinity(deltaTime) || deltaTime < 0.0f)
            {
                throw new ArgumentOutOfRangeException(nameof(deltaTime));
            }
            if (float.IsNaN(speed) || float.IsInfinity(speed))
            {
                throw new ArgumentOutOfRangeException(nameof(speed));
            }

            states[agentIndex] = JadrenAnimationState.Default;
            states[agentIndex].speed = Mathf.Max(0.0f, speed);
            states[agentIndex].lod = lod;
            if (lod == JadrenAnimationLod.Hidden)
            {
                return false;
            }

            var activeState = activeStates[agentIndex];
            var nextState = controller.ResolveState(activeState < 0 ? 0 : activeState, states[agentIndex].speed);
            if (nextState < 0)
            {
                return false;
            }
            if (nextState != activeState)
            {
                previousStates[agentIndex] = activeState;
                previousStateTimes[agentIndex] = stateTimes[agentIndex];
                activeStates[agentIndex] = nextState;
                stateTimes[agentIndex] = 0.0f;
                transitionWeights[agentIndex] = previousStates[agentIndex] < 0 ? 1.0f : 0.0f;
            }

            var definition = controller.GetState(activeStates[agentIndex]);
            if (definition.clip == null)
            {
                return false;
            }

            var playbackSpeed = Mathf.Approximately(definition.playbackSpeed, 0.0f)
                ? 1.0f
                : definition.playbackSpeed;
            var currentPreviousTime = stateTimes[agentIndex];
            var step = Mathf.Max(0.0f, deltaTime);
            if (previousStates[agentIndex] >= 0 && transitionWeights[agentIndex] < 1.0f)
            {
                var previousDefinition = controller.GetState(previousStates[agentIndex]);
                var previousPlaybackSpeed = Mathf.Approximately(previousDefinition.playbackSpeed, 0.0f)
                    ? 1.0f
                    : previousDefinition.playbackSpeed;
                previousStateTimes[agentIndex] += step * previousPlaybackSpeed;
            }
            stateTimes[agentIndex] += step * playbackSpeed;
            if (transitionWeights[agentIndex] < 1.0f)
            {
                transitionWeights[agentIndex] = Mathf.MoveTowards(
                    transitionWeights[agentIndex],
                    1.0f,
                    step / 0.15f);
            }

            states[agentIndex].stateIndex = activeStates[agentIndex];
            states[agentIndex].time = stateTimes[agentIndex];
            states[agentIndex].fadeWeight = transitionWeights[agentIndex];
            var previousState = previousStates[agentIndex];
            worker.Evaluate(
                activeStates[agentIndex],
                stateTimes[agentIndex],
                currentPreviousTime,
                previousState,
                previousStateTimes[agentIndex],
                transitionWeights[agentIndex],
                lod,
                poses[agentIndex]);

            if (transitionWeights[agentIndex] >= 1.0f && previousState >= 0)
            {
                previousStates[agentIndex] = -1;
                previousStateTimes[agentIndex] = 0.0f;
            }
            return poses[agentIndex].SampledBoneCount > 0;
        }

        /// <summary>
        /// Advances and evaluates every agent through one aggregate pose pass.
        /// The input arrays and native staging are persistent, so steady-state
        /// execution does not allocate managed frame objects.
        /// </summary>
        public int StepAll(
            float deltaTime,
            float[] speeds,
            JadrenAnimationLod lod)
        {
            ThrowIfDisposed();
            if (speeds == null) throw new ArgumentNullException(nameof(speeds));
            if (speeds.Length < poses.Length)
            {
                throw new ArgumentException("Speed array is shorter than the agent count.", nameof(speeds));
            }
            if (float.IsNaN(deltaTime) || float.IsInfinity(deltaTime) || deltaTime < 0.0f)
            {
                throw new ArgumentOutOfRangeException(nameof(deltaTime));
            }

            var step = Mathf.Max(0.0f, deltaTime);
            for (var agent = 0; agent < poses.Length; agent++)
            {
                var speed = speeds[agent];
                if (float.IsNaN(speed) || float.IsInfinity(speed))
                {
                    throw new ArgumentOutOfRangeException(nameof(speeds));
                }
                batchPrepared[agent] = false;
                batchRequests[agent] = new JadrenAnimationPoseBatchRequest
                {
                    CurrentState = -1,
                    PreviousState = -1,
                    Lod = JadrenAnimationLod.Hidden
                };
                states[agent] = JadrenAnimationState.Default;
                states[agent].speed = Mathf.Max(0.0f, speed);
                states[agent].lod = lod;
                if (lod == JadrenAnimationLod.Hidden)
                {
                    continue;
                }

                var activeState = activeStates[agent];
                var nextState = controller.ResolveState(activeState < 0 ? 0 : activeState, states[agent].speed);
                if (nextState < 0)
                {
                    continue;
                }
                if (nextState != activeState)
                {
                    previousStates[agent] = activeState;
                    previousStateTimes[agent] = stateTimes[agent];
                    activeStates[agent] = nextState;
                    stateTimes[agent] = 0.0f;
                    transitionWeights[agent] = previousStates[agent] < 0 ? 1.0f : 0.0f;
                }

                var definition = controller.GetState(activeStates[agent]);
                if (definition.clip == null)
                {
                    continue;
                }
                var playbackSpeed = Mathf.Approximately(definition.playbackSpeed, 0.0f)
                    ? 1.0f
                    : definition.playbackSpeed;
                var currentPreviousTime = stateTimes[agent];
                if (previousStates[agent] >= 0 && transitionWeights[agent] < 1.0f)
                {
                    var previousDefinition = controller.GetState(previousStates[agent]);
                    var previousPlaybackSpeed = Mathf.Approximately(previousDefinition.playbackSpeed, 0.0f)
                        ? 1.0f
                        : previousDefinition.playbackSpeed;
                    previousStateTimes[agent] += step * previousPlaybackSpeed;
                }
                stateTimes[agent] += step * playbackSpeed;
                if (transitionWeights[agent] < 1.0f)
                {
                    transitionWeights[agent] = Mathf.MoveTowards(
                        transitionWeights[agent],
                        1.0f,
                        step / 0.15f);
                }

                states[agent].stateIndex = activeStates[agent];
                states[agent].time = stateTimes[agent];
                states[agent].fadeWeight = transitionWeights[agent];
                batchRequests[agent] = new JadrenAnimationPoseBatchRequest
                {
                    CurrentState = activeStates[agent],
                    CurrentTime = stateTimes[agent],
                    CurrentPreviousTime = currentPreviousTime,
                    PreviousState = previousStates[agent],
                    PreviousTime = previousStateTimes[agent],
                    FadeWeight = transitionWeights[agent],
                    Lod = lod
                };
                batchPrepared[agent] = true;
            }

            worker.EvaluateBatch(batchRequests, poses, poses.Length);
            var updated = 0;
            for (var agent = 0; agent < poses.Length; agent++)
            {
                if (!batchPrepared[agent])
                {
                    continue;
                }
                if (transitionWeights[agent] >= 1.0f && previousStates[agent] >= 0)
                {
                    previousStates[agent] = -1;
                    previousStateTimes[agent] = 0.0f;
                }
                if (poses[agent].SampledBoneCount > 0)
                {
                    updated++;
                }
            }
            return updated;
        }

        public void ResetAgent(int agentIndex)
        {
            if (agentIndex < 0 || agentIndex >= poses.Length)
            {
                throw new ArgumentOutOfRangeException(nameof(agentIndex));
            }
            activeStates[agentIndex] = -1;
            previousStates[agentIndex] = -1;
            stateTimes[agentIndex] = 0.0f;
            previousStateTimes[agentIndex] = 0.0f;
            transitionWeights[agentIndex] = 1.0f;
            states[agentIndex] = JadrenAnimationState.Default;
        }

        public void Dispose()
        {
            disposed = true;
        }

        private void ThrowIfDisposed()
        {
            if (disposed)
            {
                throw new ObjectDisposedException(nameof(JadrenAnimationBatchPoseEvaluator));
            }
        }
    }
}
