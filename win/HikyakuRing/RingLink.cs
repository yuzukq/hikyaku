// hikyaku との BLE の通信。Windows が HID として接続中の hikyaku の、独自サービス（ring）だけを使う。
//
// 見つからない間・切断の後は、数秒ごとにペアリング済みの機器を探し直す（hikyaku が Windows とつながるのを待つ）。
// 接続するたびに HELLO（00）を送る（ファームは切断で HELLO を忘れる）。イベントは UI のスレッドで渡す。

using System;
using System.Linq;
using System.Runtime.InteropServices.WindowsRuntime;
using System.Threading;
using System.Threading.Channels;
using System.Threading.Tasks;
using System.Windows.Threading;
using Hikyaku.RingCore;
using Windows.Devices.Bluetooth;
using Windows.Devices.Bluetooth.GenericAttributeProfile;
using Windows.Devices.Enumeration;

namespace Hikyaku.Ring;

public enum LinkState
{
    Starting,
    /// <summary>hikyaku が Windows とつながっていない（または独自サービスが見えない）</summary>
    Searching,
    Connecting,
    Ready,
}

public readonly record struct LinkStatus(LinkState State, string Name = "")
{
    public bool IsReady => State == LinkState.Ready;

    public string Label => State switch
    {
        LinkState.Starting => "起動中",
        LinkState.Searching => "hikyaku を探しています（Windows とつながっているか確認）",
        LinkState.Connecting => $"{Name} に接続中",
        _ => $"{Name} と接続済み",
    };
}

public sealed class RingLink : IDisposable
{
    public event Action<LinkStatus>? StatusChanged;
    public event Action<RingEvent>? EventReceived;

    private readonly Dispatcher _ui;
    private readonly CancellationTokenSource _cts = new();
    private readonly Channel<byte[]> _outgoing = Channel.CreateUnbounded<byte[]>(new UnboundedChannelOptions { SingleReader = true });
    private GattDeviceService? _service;
    private BluetoothLEDevice? _device;
    private GattCharacteristic? _state;
    private GattCharacteristic? _ctrl;
    private LinkStatus _status = new(LinkState.Starting);

    public RingLink(Dispatcher ui) => _ui = ui;

    public LinkStatus Status => _status;

    public void Start()
    {
        _ = ConnectLoopAsync(_cts.Token);
        _ = SendLoopAsync(_cts.Token);
    }

    /// <summary>命令を送る（つながっていなければ捨てる）。送る順番は保つ。</summary>
    public void Send(byte[] command)
    {
        if (_status.IsReady) _outgoing.Writer.TryWrite(command);
    }

    private async Task SendLoopAsync(CancellationToken ct)
    {
        try
        {
            await foreach (var bytes in _outgoing.Reader.ReadAllAsync(ct))
            {
                var ctrl = _ctrl;
                if (ctrl is null) continue;
                try
                {
                    var r = await ctrl.WriteValueWithResultAsync(bytes.AsBuffer(), GattWriteOption.WriteWithResponse);
                    if (r.Status != GattCommunicationStatus.Success) Log.Error($"write {BitConverter.ToString(bytes)}: {r.Status}");
                }
                catch (Exception e)
                {
                    Log.Error($"write failed: {e.Message}");
                }
            }
        }
        catch (OperationCanceledException)
        {
        }
    }

    private async Task ConnectLoopAsync(CancellationToken ct)
    {
        while (!ct.IsCancellationRequested)
        {
            if (!_status.IsReady)
            {
                try
                {
                    await TryConnectAsync();
                }
                catch (Exception e)
                {
                    Log.Error($"connect: {e.Message}");
                    Forget();
                }
                if (!_status.IsReady) SetStatus(new LinkStatus(LinkState.Searching));
            }
            try
            {
                await Task.Delay(3000, ct);
            }
            catch (OperationCanceledException)
            {
                return;
            }
        }
    }

    private async Task TryConnectAsync()
    {
        var service = await FindServiceAsync();
        if (service is null) return;
        var device = service.Device;
        if (device is null || device.ConnectionStatus != BluetoothConnectionStatus.Connected)
        {
            service.Dispose();
            return;
        }
        var name = device.Name;
        SetStatus(new LinkStatus(LinkState.Connecting, name));
        Log.Info($"connecting to {name} ({device.BluetoothAddress:X12})");

        await service.RequestAccessAsync();
        var open = await service.OpenAsync(GattSharingMode.SharedReadAndWrite);
        if (open != GattOpenStatus.Success && open != GattOpenStatus.AlreadyOpened)
        {
            Log.Error($"open: {open}");
        }
        service.Session.MaintainConnection = true;
        var chars = await service.GetCharacteristicsAsync(BluetoothCacheMode.Uncached);
        var state = chars.Characteristics.FirstOrDefault(c => c.Uuid == RingUuid.State);
        var ctrl = chars.Characteristics.FirstOrDefault(c => c.Uuid == RingUuid.Ctrl);
        if (chars.Status != GattCommunicationStatus.Success || state is null || ctrl is null)
        {
            // Windows が古い GATT を覚えていると見えないことがある（hikyaku2 を削除して再ペアリング）
            Log.Error($"ring characteristics not found: {chars.Status}");
            service.Dispose();
            return;
        }
        state.ValueChanged += OnValueChanged;
        var cccd = await state.WriteClientCharacteristicConfigurationDescriptorAsync(
            GattClientCharacteristicConfigurationDescriptorValue.Notify);
        if (cccd != GattCommunicationStatus.Success)
        {
            Log.Error($"subscribe: {cccd}");
            state.ValueChanged -= OnValueChanged;
            service.Dispose();
            return;
        }
        _service = service;
        _device = device;
        _state = state;
        _ctrl = ctrl;
        device.ConnectionStatusChanged += OnConnectionStatusChanged;
        SetStatus(new LinkStatus(LinkState.Ready, name));
        Send(RingCommand.Hello);
        Log.Info("ready (subscribed, sent HELLO)");
    }

    /// <summary>ペアリング済みの機器から ring サービスを探す。サービスで探して見つからなければ、名前が hikyaku で始まる機器から開く。</summary>
    private static async Task<GattDeviceService?> FindServiceAsync()
    {
        var found = await DeviceInformation.FindAllAsync(GattDeviceService.GetDeviceSelectorFromUuid(RingUuid.Service));
        foreach (var info in found)
        {
            var s = await GattDeviceService.FromIdAsync(info.Id);
            if (s?.Device?.ConnectionStatus == BluetoothConnectionStatus.Connected) return s;
            s?.Dispose();
        }
        var paired = await DeviceInformation.FindAllAsync(BluetoothLEDevice.GetDeviceSelectorFromPairingState(true));
        foreach (var info in paired.Where(d => d.Name.StartsWith("hikyaku", StringComparison.OrdinalIgnoreCase)))
        {
            using var device = await BluetoothLEDevice.FromIdAsync(info.Id);
            if (device?.ConnectionStatus != BluetoothConnectionStatus.Connected) continue;
            var r = await device.GetGattServicesForUuidAsync(RingUuid.Service, BluetoothCacheMode.Uncached);
            if (r.Status == GattCommunicationStatus.Success && r.Services.Count > 0) return r.Services[0];
        }
        return null;
    }

    private void OnValueChanged(GattCharacteristic sender, GattValueChangedEventArgs args)
    {
        var ev = RingEvent.Parse(args.CharacteristicValue.ToArray());
        if (ev is null) return;
        _ui.BeginInvoke(() => EventReceived?.Invoke(ev));
    }

    private void OnConnectionStatusChanged(BluetoothLEDevice sender, object args)
    {
        if (sender.ConnectionStatus == BluetoothConnectionStatus.Connected) return;
        Log.Info("disconnected");
        Forget();
        SetStatus(new LinkStatus(LinkState.Searching));
        // 開いていたリングは、ファーム側でも切断で閉じている
        _ui.BeginInvoke(() => EventReceived?.Invoke(new RingEvent.Closed()));
    }

    private void Forget()
    {
        if (_state is not null) _state.ValueChanged -= OnValueChanged;
        if (_device is not null) _device.ConnectionStatusChanged -= OnConnectionStatusChanged;
        _service?.Dispose();
        _service = null;
        _device = null;
        _state = null;
        _ctrl = null;
    }

    private void SetStatus(LinkStatus status)
    {
        if (status == _status) return;
        _status = status;
        _ui.BeginInvoke(() => StatusChanged?.Invoke(status));
    }

    public void Dispose()
    {
        _cts.Cancel();
        Forget();
    }
}
