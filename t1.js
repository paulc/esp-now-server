
mqtt_tx(new mqtt.MqttCommand("subscribe","test/topic","qos0"));

const repeat = (t, f) => { const run = () => { f(); setTimeout(run, t); }; setTimeout(run, t); };

repeat(5,mqtt_tx(new mqtt.MqttCommand("publish","test/topic",__to_buffer("HELLO"),"qos0")));

// while (true) {
//     const m = await mqtt_rx();
//     console.log("RX >>", m.debug());
// }
