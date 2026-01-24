
fn on_init() {
    print("ON_INIT >>> ");
    let data = blob();
    data.append("BROADCAST");
    command_tx(new_broadcast(data,10));
}

fn on_event(msg) {
    let m = msg.parse();
    if m.Recv != () {
        let id = m.Recv.id;
        let mac = m.Recv.src_addr.format_mac();
        let data = m.Recv.data.to_blob().as_string();
        let rssi = m.Recv.rssi;
        print(`RECV: id=${id} mac=${mac} rssi=${rssi} data="${data}"`);
    } else {
        print(`ON_EVENT: ${m}`);
    }
}

fn on_tick(counter) {
    print(">>> ON_TICK >>> " + counter);
    let data = blob();
    data.append("TX_DATA");
    command_tx(new_txdata("98:a3:16:8e:ff:c0",data,false));
}

fn array_to_blob(a) {
    let b = blob(a.len());
    for (v,i) in a { b.set(i,v) };
    b
}
