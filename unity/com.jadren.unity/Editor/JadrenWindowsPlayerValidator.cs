using System.IO;
using UnityEditor;
using UnityEditor.Build;
using UnityEditor.Build.Reporting;

namespace Jadren.Unity.Editor
{
    /// <summary>
    /// Fails a Windows Player build early when the required native plugin pair is absent.
    /// Runtime DLLs are not silently copied or renamed by the validator.
    /// </summary>
    public sealed class JadrenWindowsPlayerValidator : IPreprocessBuildWithReport
    {
        public int callbackOrder => 0;

        public void OnPreprocessBuild(BuildReport report)
        {
            if (report.summary.platform != BuildTarget.StandaloneWindows64)
            {
                return;
            }

            var packageRoot = Path.GetFullPath("Packages/com.jadren.unity");
            if (!HasWindowsPlugin(packageRoot))
            {
                throw new BuildFailedException(
                    "Jadren Windows Player requires Plugins/x86_64/jadren_native.dll "
                    + "(generated kernels) and jadren_runtime.dll (runtime ABI).");
            }
        }

        internal static bool HasWindowsPlugin(string packageRoot)
        {
            var pluginRoot = Path.Combine(packageRoot, "Plugins", "x86_64");
            return File.Exists(Path.Combine(pluginRoot, "jadren_native.dll"))
                && File.Exists(Path.Combine(pluginRoot, "jadren_runtime.dll"));
        }
    }
}
