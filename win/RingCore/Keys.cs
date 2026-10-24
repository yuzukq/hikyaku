// キーの組み合わせ（hikyaku から送る形）と、Windows の仮想キーコード → HID の Usage ID の対応。
//
// 設定画面で押して記録するときは、Windows の仮想キーコード（VK_*）が取れる。
// hikyaku に送るのは HID の Usage ID（Keyboard/Keypad Page）なので、ここで変換する。

namespace Hikyaku.RingCore;

/// <summary>修飾キー + 1 キー。ビットはブートキーボードと同じ（bit0 Ctrl, 1 Shift, 2 Alt, 3 Win。4〜7 は右側）。</summary>
public readonly record struct KeyCombo(byte Modifiers, byte Usage)
{
    public const byte Ctrl = 0x01;
    public const byte Shift = 0x02;
    public const byte Alt = 0x04;
    public const byte Win = 0x08;

    /// <summary>「Ctrl+Shift+Win+S」のような表示。左右は区別しない。</summary>
    public string Label
    {
        get
        {
            var m = (byte)(Modifiers | Modifiers >> 4);
            var parts = new List<string>();
            if ((m & Ctrl) != 0) parts.Add("Ctrl");
            if ((m & Alt) != 0) parts.Add("Alt");
            if ((m & Shift) != 0) parts.Add("Shift");
            if ((m & Win) != 0) parts.Add("Win");
            if (Usage != 0) parts.Add(HidKey.Name(Usage));
            return parts.Count == 0 ? "なし" : string.Join("+", parts);
        }
    }
}

public static class HidKey
{
    /// <summary>Windows の仮想キーコード → HID の Usage ID。対応がなければ null。</summary>
    public static byte? FromVirtualKey(int vk) => VkToHid.TryGetValue(vk, out var u) ? u : null;

    public static string Name(byte usage) => Names.TryGetValue(usage, out var n) ? n : $"0x{usage:X2}";

    private static readonly Dictionary<int, byte> VkToHid = BuildVkToHid();
    private static readonly Dictionary<byte, string> Names = BuildNames();

    private static Dictionary<int, byte> BuildVkToHid()
    {
        var t = new Dictionary<int, byte>();
        for (var i = 0; i < 26; i++) t[0x41 + i] = (byte)(0x04 + i); // A〜Z
        t[0x30] = 0x27; // 0
        for (var i = 1; i <= 9; i++) t[0x30 + i] = (byte)(0x1D + i); // 1〜9
        for (var i = 0; i < 12; i++) t[0x70 + i] = (byte)(0x3A + i); // F1〜F12
        for (var i = 0; i < 12; i++) t[0x7C + i] = (byte)(0x68 + i); // F13〜F24
        for (var i = 1; i <= 9; i++) t[0x60 + i] = (byte)(0x58 + i); // テンキー 1〜9
        var others = new Dictionary<int, byte>
        {
            [0x0D] = 0x28, // Enter
            [0x1B] = 0x29, // Esc
            [0x08] = 0x2A, // Backspace
            [0x09] = 0x2B, // Tab
            [0x20] = 0x2C, // Space
            [0xBD] = 0x2D, // -（OEM_MINUS）
            [0xBB] = 0x2E, // =（OEM_PLUS。JIS では ;）
            [0xDB] = 0x2F, // [
            [0xDD] = 0x30, // ]
            [0xDC] = 0x31, // \
            [0xBA] = 0x33, // ;（JIS では :）
            [0xDE] = 0x34, // '
            [0xC0] = 0x35, // `（JIS では @）
            [0xBC] = 0x36, // ,
            [0xBE] = 0x37, // .
            [0xBF] = 0x38, // /
            [0x14] = 0x39, // Caps Lock
            [0x2C] = 0x46, // Print Screen
            [0x91] = 0x47, // Scroll Lock
            [0x13] = 0x48, // Pause
            [0x2D] = 0x49, // Insert
            [0x24] = 0x4A, // Home
            [0x21] = 0x4B, // Page Up
            [0x2E] = 0x4C, // Delete
            [0x23] = 0x4D, // End
            [0x22] = 0x4E, // Page Down
            [0x27] = 0x4F, // →
            [0x25] = 0x50, // ←
            [0x28] = 0x51, // ↓
            [0x26] = 0x52, // ↑
            [0x90] = 0x53, // Num Lock
            [0x6F] = 0x54, [0x6A] = 0x55, [0x6D] = 0x56, [0x6B] = 0x57, // テンキー / * - +
            [0x60] = 0x62, // テンキー 0
            [0x6E] = 0x63, // テンキー .
            [0x5D] = 0x65, // Application（メニュー）
            [0xE2] = 0x87, // JIS _（International1）
            [0x1C] = 0x8A, // 変換（International4）
            [0x1D] = 0x8B, // 無変換（International5）
            [0xF2] = 0x88, // カタカナ・ひらがな（International2）
            [0xF3] = 0x35, [0xF4] = 0x35, // 半角/全角（Zenkaku/Hankaku は ` の位置）
        };
        foreach (var (vk, u) in others) t.TryAdd(vk, u);
        return t;
    }

    private static Dictionary<byte, string> BuildNames()
    {
        var n = new Dictionary<byte, string>();
        for (var i = 0; i < 26; i++) n[(byte)(0x04 + i)] = ((char)('A' + i)).ToString();
        for (var i = 1; i <= 9; i++) n[(byte)(0x1D + i)] = i.ToString();
        n[0x27] = "0";
        for (var i = 1; i <= 12; i++) n[(byte)(0x39 + i)] = $"F{i}";
        for (var i = 13; i <= 24; i++) n[(byte)(0x68 + i - 13)] = $"F{i}";
        var others = new Dictionary<byte, string>
        {
            [0x28] = "Enter", [0x29] = "Esc", [0x2A] = "Backspace", [0x2B] = "Tab", [0x2C] = "Space",
            [0x2D] = "-", [0x2E] = "=", [0x2F] = "[", [0x30] = "]", [0x31] = "\\", [0x33] = ";", [0x34] = "'",
            [0x35] = "`", [0x36] = ",", [0x37] = ".", [0x38] = "/", [0x39] = "Caps Lock",
            [0x46] = "Print Screen", [0x47] = "Scroll Lock", [0x48] = "Pause",
            [0x49] = "Insert", [0x4A] = "Home", [0x4B] = "Page Up", [0x4C] = "Delete", [0x4D] = "End", [0x4E] = "Page Down",
            [0x4F] = "→", [0x50] = "←", [0x51] = "↓", [0x52] = "↑", [0x53] = "Num Lock",
            [0x54] = "Num /", [0x55] = "Num *", [0x56] = "Num -", [0x57] = "Num +",
            [0x59] = "Num 1", [0x5A] = "Num 2", [0x5B] = "Num 3", [0x5C] = "Num 4", [0x5D] = "Num 5",
            [0x5E] = "Num 6", [0x5F] = "Num 7", [0x60] = "Num 8", [0x61] = "Num 9", [0x62] = "Num 0", [0x63] = "Num .",
            [0x65] = "Menu", [0x87] = "_", [0x88] = "かな", [0x8A] = "変換", [0x8B] = "無変換",
        };
        foreach (var (u, s) in others) n.TryAdd(u, s);
        return n;
    }
}
