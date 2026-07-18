using System;
using System.Diagnostics;
using System.IO;
using System.Text;

namespace Jadren.Unity.Editor
{
    /// <summary>Explicit editor action that materializes bindgen C# source.</summary>
    public static class JadrenGeneratedBindings
    {
        public static bool TryGenerate(
            string compilerPath,
            string sourcePath,
            string outputPath,
            out string diagnostics)
        {
            diagnostics = string.Empty;
            if (string.IsNullOrWhiteSpace(compilerPath)
                || string.IsNullOrWhiteSpace(sourcePath)
                || string.IsNullOrWhiteSpace(outputPath))
            {
                diagnostics = "compiler, source and output paths are required";
                return false;
            }

            compilerPath = Path.GetFullPath(compilerPath);
            sourcePath = Path.GetFullPath(sourcePath);
            outputPath = Path.GetFullPath(outputPath);
            if (!File.Exists(compilerPath) || !File.Exists(sourcePath))
            {
                diagnostics = "compiler or source path does not exist";
                return false;
            }
            if (!string.Equals(Path.GetExtension(sourcePath), ".jdn", StringComparison.OrdinalIgnoreCase))
            {
                diagnostics = "source path must have the .jdn extension";
                return false;
            }
            if (!string.Equals(Path.GetExtension(outputPath), ".cs", StringComparison.OrdinalIgnoreCase))
            {
                diagnostics = "output path must have the .cs extension";
                return false;
            }

            var startInfo = new ProcessStartInfo
            {
                FileName = compilerPath,
                Arguments = "emit csharp " + QuoteArgument(sourcePath),
                UseShellExecute = false,
                CreateNoWindow = true,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
            };

            try
            {
                using (var process = new Process { StartInfo = startInfo })
                {
                    if (!process.Start())
                    {
                        diagnostics = "compiler process did not start";
                        return false;
                    }

                    var stdout = process.StandardOutput.ReadToEnd();
                    var stderr = process.StandardError.ReadToEnd();
                    process.WaitForExit();
                    if (process.ExitCode != 0)
                    {
                        diagnostics = string.IsNullOrWhiteSpace(stderr)
                            ? $"compiler exited with code {process.ExitCode}"
                            : stderr.Trim();
                        return false;
                    }
                    if (string.IsNullOrWhiteSpace(stdout))
                    {
                        diagnostics = "compiler produced an empty C# binding";
                        return false;
                    }

                    var directory = Path.GetDirectoryName(outputPath);
                    if (!string.IsNullOrEmpty(directory))
                    {
                        Directory.CreateDirectory(directory);
                    }
                    var temporaryPath = outputPath + ".tmp-" + Guid.NewGuid().ToString("N");
                    try
                    {
                        File.WriteAllText(temporaryPath, stdout, new UTF8Encoding(false));
                        if (File.Exists(outputPath))
                        {
                            File.Replace(temporaryPath, outputPath, null);
                        }
                        else
                        {
                            File.Move(temporaryPath, outputPath);
                        }
                    }
                    finally
                    {
                        if (File.Exists(temporaryPath))
                        {
                            File.Delete(temporaryPath);
                        }
                    }
                    diagnostics = stderr.Trim();
                    return true;
                }
            }
            catch (Exception exception) when (
                exception is IOException
                || exception is UnauthorizedAccessException
                || exception is System.ComponentModel.Win32Exception)
            {
                diagnostics = exception.Message;
                return false;
            }
        }

        private static string QuoteArgument(string value)
        {
            return "\"" + value.Replace("\"", "\\\"") + "\"";
        }
    }
}
