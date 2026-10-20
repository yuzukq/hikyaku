// hikyaku との BLE の通信。macOS が HID として接続中の hikyaku の、独自サービス（ring）だけを使う。
//
// 見つからない間・切断の後は、数秒ごとに接続済みの機器を探し直す（hikyaku が Mac とつながるのを待つ）。
// 接続するたびに HELLO（00）を送る（ファームは切断で HELLO を忘れる）。

import CoreBluetooth
import Foundation
import os
import RingCore

let log = Logger(subsystem: "dev.hikyaku.ring", category: "ring")

enum LinkStatus: Equatable {
    case starting
    case bluetoothOff
    case unauthorized
    /// hikyaku が Mac とつながっていない（または独自サービスが見えない）
    case searching
    case connecting(name: String)
    case ready(name: String)

    var label: String {
        switch self {
        case .starting: "起動中"
        case .bluetoothOff: "Bluetooth がオフ"
        case .unauthorized: "Bluetooth が許可されていない（システム設定 → プライバシーとセキュリティ → Bluetooth）"
        case .searching: "hikyaku を探しています"
        case .connecting(let name): "\(name) に接続中"
        case .ready(let name): "\(name) と接続済み"
        }
    }

    var isReady: Bool {
        if case .ready = self { true } else { false }
    }
}

@MainActor
final class RingLink: NSObject {
    var onStatus: (LinkStatus) -> Void = { _ in }
    var onEvent: (RingEvent) -> Void = { _ in }

    private let serviceUUID = CBUUID(string: RingUUID.service)
    private let stateUUID = CBUUID(string: RingUUID.state)
    private let ctrlUUID = CBUUID(string: RingUUID.ctrl)

    private var central: CBCentralManager!
    private var peripheral: CBPeripheral?
    private var ctrl: CBCharacteristic?
    private var searchTimer: Timer?
    private var lastSeq: UInt8?

    private(set) var status: LinkStatus = .starting {
        didSet { if status != oldValue { onStatus(status) } }
    }

    func start() {
        central = CBCentralManager(delegate: self, queue: nil)
    }

    func send(_ command: RingCommand) {
        guard let peripheral, let ctrl, status.isReady else { return }
        peripheral.writeValue(Data(command.bytes), for: ctrl, type: .withResponse)
    }

    private func searchAgainLater() {
        guard searchTimer == nil else { return }
        searchTimer = Timer.scheduledTimer(withTimeInterval: 3, repeats: true) { [weak self] _ in
            MainActor.assumeIsolated { self?.findConnectedHikyaku() }
        }
    }

    private func findConnectedHikyaku() {
        guard central.state == .poweredOn else { return }
        if let peripheral, peripheral.state == .connected || peripheral.state == .connecting { return }
        guard let p = central.retrieveConnectedPeripherals(withServices: [serviceUUID]).first else {
            status = .searching
            searchAgainLater()
            return
        }
        searchTimer?.invalidate()
        searchTimer = nil
        peripheral = p
        p.delegate = self
        status = .connecting(name: p.name ?? "hikyaku")
        log.info("connecting to \(p.name ?? "?", privacy: .public) (\(p.identifier, privacy: .public))")
        central.connect(p)
    }

    private func forget() {
        ctrl = nil
        lastSeq = nil
        peripheral = nil
    }
}

extension RingLink: CBCentralManagerDelegate {
    nonisolated func centralManagerDidUpdateState(_ central: CBCentralManager) {
        MainActor.assumeIsolated {
            switch central.state {
            case .poweredOn:
                findConnectedHikyaku()
            case .unauthorized:
                status = .unauthorized
            default:
                forget()
                status = .bluetoothOff
            }
        }
    }

    nonisolated func centralManager(_ central: CBCentralManager, didConnect peripheral: CBPeripheral) {
        MainActor.assumeIsolated {
            peripheral.discoverServices([serviceUUID])
        }
    }

    nonisolated func centralManager(_ central: CBCentralManager, didFailToConnect peripheral: CBPeripheral, error: Error?) {
        MainActor.assumeIsolated {
            log.error("connect failed: \(String(describing: error), privacy: .public)")
            forget()
            status = .searching
            searchAgainLater()
        }
    }

    nonisolated func centralManager(_ central: CBCentralManager, didDisconnectPeripheral peripheral: CBPeripheral, error: Error?) {
        MainActor.assumeIsolated {
            log.info("disconnected: \(String(describing: error), privacy: .public)")
            forget()
            status = .searching
            // 開いていたリングは、ファーム側でも切断で閉じている
            onEvent(.closed)
            searchAgainLater()
        }
    }
}

extension RingLink: CBPeripheralDelegate {
    nonisolated func peripheral(_ peripheral: CBPeripheral, didDiscoverServices error: Error?) {
        MainActor.assumeIsolated {
            guard let service = peripheral.services?.first(where: { $0.uuid == serviceUUID }) else {
                // macOS が古い GATT を覚えていると見えない（Mac で hikyaku を削除して再ペアリング）
                log.error("ring service not found: \(peripheral.services?.map(\.uuid.uuidString) ?? [], privacy: .public)")
                central.cancelPeripheralConnection(peripheral)
                return
            }
            peripheral.discoverCharacteristics([stateUUID, ctrlUUID], for: service)
        }
    }

    nonisolated func peripheral(_ peripheral: CBPeripheral, didDiscoverCharacteristicsFor service: CBService, error: Error?) {
        MainActor.assumeIsolated {
            let chars = service.characteristics ?? []
            guard let state = chars.first(where: { $0.uuid == stateUUID }),
                  let ctrl = chars.first(where: { $0.uuid == ctrlUUID })
            else {
                log.error("ring characteristics not found")
                central.cancelPeripheralConnection(peripheral)
                return
            }
            self.ctrl = ctrl
            peripheral.setNotifyValue(true, for: state)
            peripheral.writeValue(Data(RingCommand.hello.bytes), for: ctrl, type: .withResponse)
            status = .ready(name: peripheral.name ?? "hikyaku")
            log.info("ready (subscribed, sent HELLO)")
        }
    }

    nonisolated func peripheral(_ peripheral: CBPeripheral, didUpdateNotificationStateFor characteristic: CBCharacteristic, error: Error?) {
        if let error {
            log.error("subscribe failed: \(error, privacy: .public)")
        }
    }

    nonisolated func peripheral(_ peripheral: CBPeripheral, didWriteValueFor characteristic: CBCharacteristic, error: Error?) {
        if let error {
            log.error("write failed: \(error, privacy: .public)")
        }
    }

    nonisolated func peripheral(_ peripheral: CBPeripheral, didUpdateValueFor characteristic: CBCharacteristic, error: Error?) {
        MainActor.assumeIsolated {
            guard let data = characteristic.value, let event = RingEvent(data) else { return }
            if case .position(let s) = event {
                if let lastSeq, s.seq != lastSeq &+ 1 {
                    log.debug("missed \(Int(s.seq &- lastSeq) - 1) events")
                }
                lastSeq = s.seq
            }
            onEvent(event)
        }
    }
}
