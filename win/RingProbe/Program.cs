// hikyaku のショートカットリング: Windows 側の試作（最大のリスクの確認用）。
//
// 確かめること: Windows が、HID（キーボード・マウス）として接続中の hikyaku の独自サービス（ring）を、
// アプリから開かせるか（Mac では開けた。mac/RingProbe）。
//
// 使い方（Windows で。hikyaku2 を Windows とペアリングして、普通につながった状態のまま）:
//   RingProbe.exe
// 入力: o + Enter = 開く、c + Enter = 閉じる、q + Enter = 終了。F23 でも開閉できる。
//       + / - + Enter = 音量（hikyaku からメディアキーを送る）、k + Enter = Win+S（検索。hikyaku からキーを送る）
//       s + Enter = 設定を問い合わせる
//
// 仕様: docs/design/ring.md、イベントの形式: crates/hikyaku-pointer/src/ring.rs

using System.Runtime.InteropServices.WindowsRuntime;
using Hikyaku.RingCore;
using Windows.Devices.Bluetooth;
using Windows.Devices.Bluetooth.GenericAttributeProfile;
using Windows.Devices.Enumeration;

Console.OutputEncoding = System.Text.Encoding.UTF8;
try
{
    return await Probe();
}
catch (Exception e)
{
    Console.WriteLine($"例外: {e}");
    return 1;
}
finally
{
    // ダブルクリックで開いたとき、結果を読めるように閉じずに待つ
    Console.WriteLine();
    Console.WriteLine("Enter で閉じます");
    Console.ReadLine();
}

async Task<int> Probe()
{

// 1. ペアリング済みの機器のうち、ring サービスを持つもの（これが取れるかが最大の確認点）
var selector = GattDeviceService.GetDeviceSelectorFromUuid(RingUuid.Service);
var found = await DeviceInformation.FindAllAsync(selector);
Console.WriteLine($"ring サービスを持つ機器: {found.Count} 件 {string.Join(", ", found.Select(d => d.Name))}");

// 見つからなければ、ペアリング済みの BLE 機器の名前で探して、そこから ring サービスを開く
GattDeviceService? service = null;
if (found.Count > 0)
{
    service = await GattDeviceService.FromIdAsync(found[0].Id);
    Console.WriteLine(service is null ? "サービスを開けなかった（FromIdAsync が null）" : "サービスを開いた（サービスの ID から）");
}
if (service is null)
{
    var paired = await DeviceInformation.FindAllAsync(BluetoothLEDevice.GetDeviceSelectorFromPairingState(true));
    Console.WriteLine($"ペアリング済みの BLE 機器: {string.Join(", ", paired.Select(d => d.Name))}");
    var info = paired.FirstOrDefault(d => d.Name.StartsWith("hikyaku", StringComparison.OrdinalIgnoreCase));
    if (info is null)
    {
        Console.WriteLine("hikyaku が見つからない。Windows の設定 → Bluetooth で hikyaku2 とペアリングしているか確認する");
        return 1;
    }
    Console.WriteLine($"ID: {info.Id}");
    Console.WriteLine($"有効: {info.IsEnabled}、ペアリング: {info.Pairing.IsPaired}");
    foreach (var (k, v) in info.Properties) Console.WriteLine($"  {k} = {v}");
    BluetoothLEDevice? device = null;
    try
    {
        device = await BluetoothLEDevice.FromIdAsync(info.Id);
    }
    catch (Exception e)
    {
        Console.WriteLine($"ID から開けない: {e.GetType().Name}: {e.Message}");
    }
    // ID で開けなければ、アドレスから（1 台目 hikyaku = FF:E4:05:1A:8F:FF、2 台目 hikyaku2 = FF:E4:05:1A:8F:FE）
    foreach (var address in new ulong[] { 0xFFE4051A8FFE, 0xFFE4051A8FFF })
    {
        if (device is not null) break;
        try
        {
            device = await BluetoothLEDevice.FromBluetoothAddressAsync(address, BluetoothAddressType.Random);
            Console.WriteLine($"アドレス {address:X12} から: {(device is null ? "見つからない" : "開いた")}");
        }
        catch (Exception e)
        {
            Console.WriteLine($"アドレス {address:X12} から開けない: {e.GetType().Name}: {e.Message}");
        }
    }
    if (device is null) return 1;
    Console.WriteLine($"{device.Name}（{device.BluetoothAddress:X12}）: 接続 {device.ConnectionStatus}");
    var all = await device.GetGattServicesAsync(BluetoothCacheMode.Uncached);
    Console.WriteLine($"GATT サービス（{all.Status}）: {string.Join(", ", all.Services.Select(s => s.Uuid.ToString()[..8]))}");
    var r = await device.GetGattServicesForUuidAsync(RingUuid.Service, BluetoothCacheMode.Uncached);
    Console.WriteLine($"ring サービス: {r.Status}、{r.Services.Count} 件");
    service = r.Services.FirstOrDefault();
    if (service is null)
    {
        Console.WriteLine("ring サービスが見えない。Windows が HID の機器の独自サービスを隠しているか、GATT の情報が古い（hikyaku2 を削除してペアリングし直す）");
        return 1;
    }
}

Console.WriteLine($"機器: {service.Device?.Name}");
var access = await service.RequestAccessAsync();
Console.WriteLine($"アクセス: {access}");
var open = await service.OpenAsync(GattSharingMode.SharedReadAndWrite);
Console.WriteLine($"開く: {open}");
service.Session.MaintainConnection = true;

var chars = await service.GetCharacteristicsAsync(BluetoothCacheMode.Uncached);
Console.WriteLine($"キャラクタリスティック（{chars.Status}）: {chars.Characteristics.Count} 件");
var state = chars.Characteristics.FirstOrDefault(c => c.Uuid == RingUuid.State);
var ctrl = chars.Characteristics.FirstOrDefault(c => c.Uuid == RingUuid.Ctrl);
if (state is null || ctrl is null)
{
    Console.WriteLine("state / ctrl が見つからない");
    return 1;
}

var count = 0;
int? lastSector = null;
state.ValueChanged += (_, args) =>
{
    var ev = RingEvent.Parse(args.CharacteristicValue.ToArray());
    switch (ev)
    {
        case RingEvent.Opened: Console.WriteLine("開いた"); break;
        case RingEvent.Closed: Console.WriteLine("閉じた"); break;
        case RingEvent.Settings s: Console.WriteLine($"設定: 置き方 {s.Rotation.Title()}、送り先 {s.Profile + 1}、スクロール反転 {s.ScrollInvert}"); break;
        case RingEvent.Position(var p):
            count++;
            if (p.Touching)
            {
                lastSector = p.Radius < 350 ? null : (int)((p.Angle + 22.5) % 360 / 45);
                if (count % 5 == 0)
                    Console.WriteLine($"角度 {p.Angle,6:F1}°  距離 {p.Radius,4}  項目 {(lastSector?.ToString() ?? "-")}  (x={p.X} y={p.Y} z={p.Z})");
            }
            else if (lastSector is not null || count > 0)
            {
                Console.WriteLine(lastSector is { } sct ? $"離した → 項目 {sct} を実行" : "離した → キャンセル");
                lastSector = null;
                count = 0;
            }
            break;
    }
};
var cccd = await state.WriteClientCharacteristicConfigurationDescriptorAsync(GattClientCharacteristicConfigurationDescriptorValue.Notify);
Console.WriteLine($"購読: {cccd}");

async Task Send(byte[] bytes)
{
    var result = await ctrl.WriteValueWithResultAsync(bytes.AsBuffer(), GattWriteOption.WriteWithResponse);
    if (result.Status != GattCommunicationStatus.Success) Console.WriteLine($"書き込み失敗: {result.Status}");
}

await Send(RingCommand.Hello);
Console.WriteLine("HELLO を送った。F23・本体のボタンのダブルタップ・「o + Enter」でリングを開く");

while (Console.ReadLine() is { } line)
{
    switch (line.Trim())
    {
        case "o": await Send(RingCommand.Open); break;
        case "c": await Send(RingCommand.Close); break;
        case "+": await Send(RingCommand.Consumer(ConsumerUsage.VolumeUp)); break;
        case "-": await Send(RingCommand.Consumer(ConsumerUsage.VolumeDown)); break;
        case "k": await Send(RingCommand.Key(new KeyCombo(KeyCombo.Win, 0x16))); break; // Win+S
        case "s": await Send(RingCommand.GetSettings); break;
        case "q":
            await Send(RingCommand.Close);
            return 0;
    }
}
return 0;
}
