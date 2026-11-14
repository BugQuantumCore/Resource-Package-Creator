// mrpc_script.hpp — C++ SDK for native train scripts in the
// MTR Resource Package Creator *preview runtime*.
//
// This mirrors the create / render / dispose lifecycle and wrapper vocabulary
// of the Joban Client Mod (Yanyang) native SDK (`mtr::VehicleScript<State>`,
// `MTR_REGISTER_VEHICLE_SCRIPT`) so a script's structure ports 1:1 to the game.
// The ABI below is the PREVIEW ABI of this tool (abi = 1); JCM in-game uses
// its own ABI 6+ bridge — recompile against the JCM SDK for real gameplay.
//
// Usage:
//   struct State { int ticks; };
//   struct MyLcd : mrpc::VehicleScript<State> {
//       static constexpr auto ID = "my_pack:my_lcd";
//       void create(const mrpc::VehicleSnapshot& s, State& st) override { st.ticks = 0; }
//       void render(const mrpc::VehicleSnapshot& s, State& st) override {
//           st.ticks++;
//           host().display("Speed: " + std::to_string((int)s.speed_kmh) + " km/h");
//       }
//   };
//   MRPC_REGISTER_VEHICLE_SCRIPT(MyLcd)
//
// Compile into a shared library and reference it from a `vehicleScripts`
// entry's `scriptLocations` (e.g. `mtr:my_pack/natives/my_lcd`); the creator
// loads it through `mrpc-script`'s native preview host for testing.

#pragma once

#include <cstdint>
#include <string>

namespace mrpc {

// Direction-of-door-opening vocabulary shared with the JS wrapper:
// 0 = "not", 1 = "left", 2 = "right", 3 = "all".
enum class DoorDirection : int8_t { Not = 0, Left = 1, Right = 2, All = 3 };

// Per-frame vehicle data, the same fields the JS VehicleWrapper exposes.
struct VehicleSnapshot {
    double speed_kmh;
    double speed_ms;
    double door_value;          // 0 closed .. 1 open
    int door_opening;           // 1 while opening, 0 otherwise
    int notch;                  // traction notch, negative = braking
    int car_count;
    DoorDirection door_direction;
    int reversed;
    int on_route;
    int64_t total_dwell_ms;
    int64_t elapsed_dwell_ms;
};

// Host services provided by the preview runtime.
class Host {
public:
    virtual ~Host() = default;
    virtual void log(const std::string& message) = 0;
    virtual void display(const std::string& text) = 0;
};

// Base class for vehicle scripts. `State` is constructed before `create` and
// destroyed after `dispose`; members of the script class itself are shared by
// every instance of the module.
template <typename State>
class VehicleScript {
public:
    virtual ~VehicleScript() = default;
    virtual void create(const VehicleSnapshot& snapshot, State& state) { (void)snapshot; (void)state; }
    virtual void render(const VehicleSnapshot& snapshot, State& state) { (void)snapshot; (void)state; }
    virtual void dispose(const VehicleSnapshot& snapshot, State& state) { (void)snapshot; (void)state; }
    Host& host() { return *host_; }
    void set_host(Host* host) { host_ = host; }

private:
    Host* host_ = nullptr;
};

} // namespace mrpc

// The exports the preview host resolves with dlopen/LoadLibrary.
// Defined by MRPC_REGISTER_VEHICLE_SCRIPT below. The registered CLASS must
// declare `using State = ...;` (the per-instance data type it is templated on).
extern "C" {
int mrpc_module_abi();
const char* mrpc_module_id();
void* mrpc_module_create(const mrpc::VehicleSnapshot* snapshot, mrpc::Host* host);
void mrpc_module_render(void* instance, const mrpc::VehicleSnapshot* snapshot);
void mrpc_module_dispose(void* instance, const mrpc::VehicleSnapshot* snapshot);
}

#define MRPC_REGISTER_VEHICLE_SCRIPT(CLASS)                                                  \
    using MrpcStateOf_##CLASS = typename CLASS::State;                                       \
    struct MrpcPairOf_##CLASS { CLASS* script; MrpcStateOf_##CLASS* state; };                \
    extern "C" int mrpc_module_abi() { return 1; }                                           \
    extern "C" const char* mrpc_module_id() { return CLASS::ID; }                             \
    extern "C" void* mrpc_module_create(const mrpc::VehicleSnapshot* snapshot, mrpc::Host* host) { \
        auto* script = new CLASS();                                                           \
        script->set_host(host);                                                               \
        auto* state = new MrpcStateOf_##CLASS();                                              \
        script->create(*snapshot, *state);                                                    \
        return new MrpcPairOf_##CLASS{script, state};                                        \
    }                                                                                         \
    extern "C" void mrpc_module_render(void* instance, const mrpc::VehicleSnapshot* snapshot) { \
        auto* pair = static_cast<MrpcPairOf_##CLASS*>(instance);                              \
        pair->script->render(*snapshot, *pair->state);                                        \
    }                                                                                         \
    extern "C" void mrpc_module_dispose(void* instance, const mrpc::VehicleSnapshot* snapshot) { \
        auto* pair = static_cast<MrpcPairOf_##CLASS*>(instance);                              \
        pair->script->dispose(*snapshot, *pair->state);                                       \
        delete pair->script;                                                                  \
        delete pair->state;                                                                   \
        delete pair;                                                                          \
    }
