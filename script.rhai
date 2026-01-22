
fn on_init() {
    print("ON_INIT >>> ");
    let data = blob();
    data.append("BROADCAST");
    command_tx(new_broadcast(data,5));
}

fn on_event(msg) {
    print("ON_EVENT >>> " + msg.parse());
}

fn on_tick(counter) {
    print(">>> ON_TICK >>> " + counter);
    let data = blob();
    data.append("TX_DATA");
    command_tx(new_txdata("98:88:e0:7f:09:70",data,false));
}

