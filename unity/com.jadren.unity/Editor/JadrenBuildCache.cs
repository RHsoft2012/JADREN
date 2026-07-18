using System;
using System.Security.Cryptography;
using System.Text;
using UnityEditor;

namespace Jadren.Unity.Editor
{
    /// <summary>Explicit targets supported by the first Unity build matrix.</summary>
    public enum JadrenTarget
    {
        WindowsX64,
        LinuxX64,
        MacOS,
        AndroidArm64,
        IOS,
    }

    /// <summary>All inputs that can change a native Jadren artifact.</summary>
    public readonly struct JadrenBuildKey : IEquatable<JadrenBuildKey>
    {
        public JadrenBuildKey(string sourceHash, string compilerVersion, JadrenTarget target, string flags)
        {
            SourceHash = sourceHash ?? throw new ArgumentNullException(nameof(sourceHash));
            CompilerVersion = compilerVersion ?? throw new ArgumentNullException(nameof(compilerVersion));
            Target = target;
            Flags = flags ?? string.Empty;
        }

        public string SourceHash { get; }
        public string CompilerVersion { get; }
        public JadrenTarget Target { get; }
        public string Flags { get; }

        public bool Equals(JadrenBuildKey other)
        {
            return SourceHash == other.SourceHash
                && CompilerVersion == other.CompilerVersion
                && Target == other.Target
                && Flags == other.Flags;
        }

        public override bool Equals(object obj) => obj is JadrenBuildKey other && Equals(other);
        public override int GetHashCode() => HashCode.Combine(SourceHash, CompilerVersion, Target, Flags);
    }

    /// <summary>Pure cache-key and Unity-target mapping helpers.</summary>
    public static class JadrenBuildCache
    {
        public static string HashSource(string source)
        {
            if (source == null)
            {
                throw new ArgumentNullException(nameof(source));
            }

            using (var sha256 = SHA256.Create())
            {
                var bytes = sha256.ComputeHash(Encoding.UTF8.GetBytes(source));
                var builder = new StringBuilder(bytes.Length * 2);
                foreach (var value in bytes)
                {
                    builder.Append(value.ToString("x2"));
                }
                return builder.ToString();
            }
        }

        public static string CacheFileName(JadrenBuildKey key)
        {
            var identity = string.Concat(
                key.SourceHash,
                "\n",
                key.CompilerVersion,
                "\n",
                key.Target,
                "\n",
                key.Flags);
            return "jdn-" + HashSource(identity) + ".artifact";
        }

        public static bool TryGetTarget(BuildTarget buildTarget, out JadrenTarget target)
        {
            switch (buildTarget)
            {
                case BuildTarget.StandaloneWindows64:
                    target = JadrenTarget.WindowsX64;
                    return true;
                case BuildTarget.StandaloneLinux64:
                    target = JadrenTarget.LinuxX64;
                    return true;
                case BuildTarget.StandaloneOSX:
                    target = JadrenTarget.MacOS;
                    return true;
                case BuildTarget.Android:
                    target = JadrenTarget.AndroidArm64;
                    return true;
                case BuildTarget.iOS:
                    target = JadrenTarget.IOS;
                    return true;
                default:
                    target = default(JadrenTarget);
                    return false;
            }
        }
    }
}
