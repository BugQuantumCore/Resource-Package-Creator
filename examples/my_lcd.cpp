// Example native vehicle script for the MTR Resource Package Creator
// preview SDK (crates/mrpc-script/cpp/mrpc_script.hpp).
//
// Compile (Linux/macOS):   g++ -std=c++17 -O2 -shared -fPIC my_lcd.cpp -o my_lcd.so
// Compile (Windows):       cl /std:c++17 /LD my_lcd.cpp /Fe:my_lcd.dll
//
// Then add a vehicleScripts entry to mtr_custom_resources.json:
//   "vehicleScripts": [{
//       "id": "my_pack:my_lcd",
//       "scriptLocations": ["mtr:my_pack/natives/my_lcd"]
//   }]
// and reference it from a vehicle via "scriptId". In the creator, use the
// Script tab's native module tester to load the compiled library.

#include "mrpc_script.hpp"
#include <string>

struct State {
    int frames = 0;
    double lastDoor = 0;
};

struct MyLcd : mrpc::VehicleScript<State> {
    static constexpr auto ID = "my_pack:my_lcd";
    using State = ::State;

    void create(const mrpc::VehicleSnapshot& s, State& st) override {
        st.frames = 0;
        host().log("native script created, cars: " + std::to_string(s.car_count));
    }

    void render(const mrpc::VehicleSnapshot& s, State& st) override {
        st.frames++;
        if (s.door_value != st.lastDoor && s.door_value > 0) {
            host().log("doors opening");
        }
        st.lastDoor = s.door_value;
        if (st.frames % 60 == 0) {
            host().display("Speed: " + std::to_string((int)s.speed_kmh) + " km/h  Door: "
                           + std::to_string((int)(s.door_value * 100)) + "%");
        }
    }

    void dispose(const mrpc::VehicleSnapshot& s, State& st) override {
        host().log("native script disposed after " + std::to_string(st.frames) + " frames");
    }
};

MRPC_REGISTER_VEHICLE_SCRIPT(MyLcd)
