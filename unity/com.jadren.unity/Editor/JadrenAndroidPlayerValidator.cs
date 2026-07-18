using System.IO;
using UnityEditor;
using UnityEditor.Build;
using UnityEditor.Build.Reporting;

namespace Jadren.Unity.Editor
{
    /// <summary>
    /// Fails an Android Player build early when the ARM64 native plugin pair is absent.
    /// Runtime shared libraries are not silently copied or renamed by the validator.
    /// </summary>
    public sealed class JadrenAndroidPlayerValidator : IPreprocessBuildWithReport
    {
        public int callbackOrder => 1;

        public void OnPreprocessBuild(BuildReport report)
        {
            if (report.summary.platform != BuildTarget.Android)
            {
                return;
            }

            var packageRoot = Path.GetFullPath("Packages/com.jadren.unity");
            if (!HasAndroidArm64Plugin(packageRoot))
            {
                throw new BuildFailedException(
                    "Jadren Android ARM64 Player requires "
                    + "Plugins/Android/ARM64/libjadren_native.so "
                    + "(generated kernels) and libjadren_runtime.so (runtime ABI).");
            }
        }

        internal static bool HasAndroidArm64Plugin(string packageRoot)
        {
            var pluginRoot = Path.Combine(packageRoot, "Plugins", "Android", "ARM64");
            return File.Exists(Path.Combine(pluginRoot, "libjadren_native.so"))
                && File.Exists(Path.Combine(pluginRoot, "libjadren_runtime.so"));
        }
    }
}
