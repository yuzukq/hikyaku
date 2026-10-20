// hikyaku のショートカットリング: Mac 側の試作（最大のリスクの確認用）。
//
// 確かめること: macOS が HID（キーボード・マウス）として接続中の hikyaku の独自サービス（ring）に、
// アプリから CoreBluetooth でアクセスさせるか。
//
// 使い方（Mac のターミナルで。hikyaku は Mac と普通につながった状態のまま）:
//   swift mac/RingProbe/main.swift
// 初回はターミナルに Bluetooth の許可を求められる（システム設定 → プライバシーとセキュリティ → Bluetooth）。
// 入力: o + Enter = 開く、c + Enter = 閉じる、q + Enter = 終了。F23 でも開閉できる。
//       + / - + Enter = 音量（hikyaku からメディアキーを送る）、k + Enter = Command+Space（hikyaku からキーを送る）
//
// 仕様: docs/design/ring.md、イベントの形式: crates/hikyaku-pointer/src/ring.rs

import CoreBluetooth
import Foundation

let ringService = CBUUID(string: "6b9a0010-6f1e-4b8e-9d57-7f0d0a5b0001")
let stateChar = CBUUID(string: "6b9a0011-6f1e-4b8e-9d57-7f0d0a5b0001")
let ctrlChar = CBUUID(string: "6b9a0012-6f1e-4b8e-9d57-7f0d0a5b0001")
let hidService = CBUUID(string: "1812")

final class Probe: NSObject, CBCentralManagerDelegate, CBPeripheralDelegate {
    var central: CBCentralManager!
    var peripheral: CBPeripheral?
    var ctrl: CBCharacteristic?
    var count = 0
    var lastSector: Int?

    override init() {
        super.init()
        central = CBCentralManager(delegate: self, queue: nil)
    }

    func centralManagerDidUpdateState(_ central: CBCentralManager) {
        guard central.state == .poweredOn else {
            print("Bluetooth: \(central.state.rawValue)（4 = 許可されていない。ターミナルに Bluetooth を許可する）")
            return
        }
        // 1. HID として接続済みの hikyaku を、独自サービスの UUID で探す（これが取れるかが最大の確認点）
        let byRing = central.retrieveConnectedPeripherals(withServices: [ringService])
        let byHid = central.retrieveConnectedPeripherals(withServices: [hidService])
        print("接続済み（ring サービスで検索）: \(byRing.map { $0.name ?? "?" })")
        print("接続済み（HID サービスで検索）:  \(byHid.map { $0.name ?? "?" })")
        guard let p = byRing.first ?? byHid.first(where: { $0.name?.hasPrefix("hikyaku") == true }) else {
            print("hikyaku が見つからない。Mac と接続しているか確認する（ring で見つからないなら、Mac で hikyaku を削除して再ペアリングすると、新しいサービスが見えることがある）")
            exit(1)
        }
        peripheral = p
        p.delegate = self
        print("\(p.name ?? "?") に接続します（\(p.identifier)）")
        central.connect(p)
    }

    func centralManager(_ central: CBCentralManager, didConnect peripheral: CBPeripheral) {
        print("接続した。サービスを探します")
        peripheral.discoverServices([ringService])
    }

    func centralManager(_ central: CBCentralManager, didFailToConnect peripheral: CBPeripheral, error: Error?) {
        print("接続に失敗: \(String(describing: error))")
        exit(1)
    }

    func peripheral(_ peripheral: CBPeripheral, didDiscoverServices error: Error?) {
        guard let s = peripheral.services?.first(where: { $0.uuid == ringService }) else {
            print("ring サービスが見つからない（\(peripheral.services?.map { $0.uuid.uuidString } ?? [])）。")
            print("→ Mac が古い GATT の情報を覚えている可能性。Mac の Bluetooth 設定で hikyaku を削除して、ペアリングし直す")
            exit(1)
        }
        print("ring サービスが見つかった")
        peripheral.discoverCharacteristics([stateChar, ctrlChar], for: s)
    }

    func peripheral(_ peripheral: CBPeripheral, didDiscoverCharacteristicsFor service: CBService, error: Error?) {
        for c in service.characteristics ?? [] {
            if c.uuid == stateChar {
                peripheral.setNotifyValue(true, for: c)
            } else if c.uuid == ctrlChar {
                ctrl = c
                peripheral.writeValue(Data([0x00]), for: c, type: .withResponse) // HELLO
            }
        }
        print("購読して HELLO を送った。F23 か「o + Enter」でリングを開く")
    }

    func peripheral(_ peripheral: CBPeripheral, didUpdateNotificationStateFor characteristic: CBCharacteristic, error: Error?) {
        if let error { print("購読に失敗: \(error)") } else { print("購読 OK") }
    }

    func peripheral(_ peripheral: CBPeripheral, didUpdateValueFor characteristic: CBCharacteristic, error: Error?) {
        guard let d = characteristic.value, d.count >= 12 else { return }
        let b = [UInt8](d)
        let u16 = { (i: Int) in Int(b[i]) | Int(b[i + 1]) << 8 }
        switch b[0] {
        case 1: print("開いた")
        case 2: print("閉じた")
        case 3:
            count += 1
            let touching = b[1] & 1 != 0
            let angle = Double(u16(2)) / 100
            let radius = u16(4)
            if touching {
                lastSector = radius < 350 ? nil : Int(((angle + 22.5).truncatingRemainder(dividingBy: 360)) / 45)
                if count % 5 == 0 {
                    print(String(format: "角度 %6.1f°  距離 %4d  項目 %@  (x=%d y=%d z=%d)",
                                 angle, radius, lastSector.map(String.init) ?? "-", u16(6), u16(8), Int(b[10])))
                }
            } else {
                print(lastSector.map { "離した → 項目 \($0) を実行" } ?? "離した → キャンセル")
                lastSector = nil
            }
        default: break
        }
    }

    func send(_ bytes: UInt8...) {
        guard let p = peripheral, let c = ctrl else { return }
        p.writeValue(Data(bytes), for: c, type: .withResponse)
    }
}

let probe = Probe()
DispatchQueue.global().async {
    while let line = readLine() {
        DispatchQueue.main.async {
            switch line {
            case "o": probe.send(0x02)
            case "c": probe.send(0x01)
            case "q": probe.send(0x01); exit(0)
            case "+": probe.send(0x11, 0xE9, 0x00) // 音量を上げる
            case "-": probe.send(0x11, 0xEA, 0x00) // 音量を下げる
            case "k": probe.send(0x10, 0x08, 0x2C) // Command + Space
            default: break
            }
        }
    }
}
RunLoop.main.run()
