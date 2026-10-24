// ログ: %LOCALAPPDATA%\HikyakuRing\log.txt（1MB を超えたら作り直す）。不具合のときは、これを見せてもらう。

using System;
using System.IO;

namespace Hikyaku.Ring;

public static class Log
{
    private static readonly object Gate = new();
    public static readonly string PathName = System.IO.Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "HikyakuRing", "log.txt");

    public static void Info(string message) => Write("INFO", message);
    public static void Error(string message) => Write("ERROR", message);

    private static void Write(string level, string message)
    {
        try
        {
            lock (Gate)
            {
                Directory.CreateDirectory(System.IO.Path.GetDirectoryName(PathName)!);
                var info = new FileInfo(PathName);
                if (info.Exists && info.Length > 1_000_000) info.Delete();
                File.AppendAllText(PathName, $"{DateTime.Now:yyyy-MM-dd HH:mm:ss.fff} {level} {message}{Environment.NewLine}");
            }
        }
        catch (Exception)
        {
            // ログが書けなくても動き続ける
        }
    }
}
