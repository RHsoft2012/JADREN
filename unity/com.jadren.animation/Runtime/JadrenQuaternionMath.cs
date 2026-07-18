using UnityEngine;

namespace Jadren.Animation
{
    /// <summary>
    /// Worker-safe value math for the Jadren quaternion contract. It mirrors
    /// Unity's unclamped shortest-arc Slerp formula but never touches a
    /// Transform, Animator, asset or other Unity object.
    /// </summary>
    public static class JadrenQuaternionMath
    {
        public static Quaternion SlerpUnclamped(Quaternion a, Quaternion b, float t)
        {
            var dot = Quaternion.Dot(a, b);
            if (dot < 0.0f)
            {
                b = new Quaternion(-b.x, -b.y, -b.z, -b.w);
                dot = -dot;
            }

            dot = Mathf.Clamp(dot, -1.0f, 1.0f);
            if (dot > 0.9995f)
            {
                return Normalize(LerpUnclamped(a, b, t));
            }

            var theta0 = Mathf.Acos(dot);
            var sinTheta0 = Mathf.Sin(theta0);
            if (Mathf.Abs(sinTheta0) < 0.000001f)
            {
                return Normalize(LerpUnclamped(a, b, t));
            }

            var theta = theta0 * t;
            var sinTheta = Mathf.Sin(theta);
            var s0 = Mathf.Cos(theta) - dot * sinTheta / sinTheta0;
            var s1 = sinTheta / sinTheta0;
            return new Quaternion(
                a.x * s0 + b.x * s1,
                a.y * s0 + b.y * s1,
                a.z * s0 + b.z * s1,
                a.w * s0 + b.w * s1);
        }

        private static Quaternion LerpUnclamped(Quaternion a, Quaternion b, float t)
        {
            return new Quaternion(
                a.x + (b.x - a.x) * t,
                a.y + (b.y - a.y) * t,
                a.z + (b.z - a.z) * t,
                a.w + (b.w - a.w) * t);
        }

        private static Quaternion Normalize(Quaternion value)
        {
            var magnitude = Mathf.Sqrt(
                value.x * value.x + value.y * value.y + value.z * value.z + value.w * value.w);
            if (magnitude > 0.000001f)
            {
                var inverse = 1.0f / magnitude;
                return new Quaternion(
                    value.x * inverse,
                    value.y * inverse,
                    value.z * inverse,
                    value.w * inverse);
            }
            return Quaternion.identity;
        }
    }
}
