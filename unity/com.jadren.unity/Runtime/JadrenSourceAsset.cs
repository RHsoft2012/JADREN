using UnityEngine;

namespace Jadren.Unity
{
    /// <summary>Managed representation used by the future .jdn ScriptedImporter.</summary>
    public sealed class JadrenSourceAsset : ScriptableObject
    {
        [SerializeField]
        private string sourceText = string.Empty;

        public string SourceText => sourceText;

        public void SetSourceText(string value)
        {
            sourceText = value ?? string.Empty;
        }
    }
}
