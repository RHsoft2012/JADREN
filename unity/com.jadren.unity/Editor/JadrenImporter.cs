using System.IO;
using System.Text;
using UnityEditor.AssetImporters;
using UnityEngine;

namespace Jadren.Unity.Editor
{
    /// <summary>Imports UTF-8 Jadren source as a managed asset without executing it.</summary>
    [ScriptedImporter(1, "jdn")]
    public sealed class JadrenImporter : ScriptedImporter
    {
        public override void OnImportAsset(AssetImportContext context)
        {
            string source;
            try
            {
                source = File.ReadAllText(context.assetPath, new UTF8Encoding(false, true));
            }
            catch (DecoderFallbackException exception)
            {
                context.LogImportError($"Jadren source is not valid UTF-8: {exception.Message}", null);
                return;
            }
            catch (IOException exception)
            {
                context.LogImportError($"Jadren source could not be read: {exception.Message}", null);
                return;
            }

            var asset = ScriptableObject.CreateInstance<JadrenSourceAsset>();
            asset.name = Path.GetFileNameWithoutExtension(context.assetPath);
            asset.SetSourceText(source);
            context.AddObjectToAsset("source", asset);
            context.SetMainObject(asset);
        }
    }
}
