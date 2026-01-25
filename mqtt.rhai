
fn mqtt_init() {
    print(">>> MQTT_INIT");
}

fn mqtt_tick(t) {
    print(">>> MQTT_TICK: " + t);
    let data = blob();
    data.append(`TICK: ${t}`);
    mqtt_cmd(publish_msg("test/test1",data,"qos0"));
    mqtt_cmd(publish_msg("test/test2",data,"qos1"));
    if t > 5 {
        mqtt_cmd(disconnect_msg());
    }
}

fn mqtt_event(e) {
    print(">>> MQTT_EVENT: " + e);
    let event = e.parse();
    if event == "Connected" {
        print("Connected - subscribing to channels");
        mqtt_cmd(subscribe_msg("test/test1","atmostonce"));
        mqtt_cmd(subscribe_msg("test/test2","atleastonce"));
        mqtt_cmd(subscribe_msg("test/test3","exactlyonce"));
    } else {
        print("EVENT: " + event);
        if event.MessageReceived != () {
            print(">>> " + to_blob(event.MessageReceived.payload).as_string());
        }
    }
}

fn to_blob(a) {
    let b = blob(a.len());
    for (v,i) in a { b.set(i,v) };
    b
}
