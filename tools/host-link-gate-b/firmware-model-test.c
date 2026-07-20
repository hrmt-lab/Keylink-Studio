#include <assert.h>
#include <stdbool.h>
#include <stdint.h>

#include <rawhid_app/packet.h>

#include "ai_client_state_model.h"

static struct rawhid_app_ai_client_state state(uint8_t activity, uint16_t revision) {
    return (struct rawhid_app_ai_client_state){
        .client_type = RAWHID_APP_AI_CLIENT_CODEX,
        .client_variant = 0x01,
        .session_active = activity != RAWHID_APP_AI_ACTIVITY_NONE,
        .activity_state = activity,
        .revision = revision,
    };
}

int main(void) {
    struct rawhid_app_ai_client_state_model model = {0};

    struct rawhid_app_ai_client_state invalid = state(RAWHID_APP_AI_ACTIVITY_WORKING, 1);
    invalid.session_active = false;
    assert(rawhid_app_ai_client_state_model_apply(&model, &invalid) ==
           RAWHID_APP_AI_CLIENT_REJECTED);
    assert(!model.valid && model.generation == 0);

    struct rawhid_app_ai_client_state first =
        state(RAWHID_APP_AI_ACTIVITY_WORKING, 60000);
    assert(rawhid_app_ai_client_state_model_apply(&model, &first) ==
           RAWHID_APP_AI_CLIENT_UPDATED);
    assert(model.valid && model.state.revision == 60000 && model.generation == 1);

    struct rawhid_app_ai_client_state reverse =
        state(RAWHID_APP_AI_ACTIVITY_AVAILABLE, 1);
    assert(rawhid_app_ai_client_state_model_apply(&model, &reverse) ==
           RAWHID_APP_AI_CLIENT_UPDATED);
    assert(model.state.revision == 1 &&
           model.state.activity_state == RAWHID_APP_AI_ACTIVITY_AVAILABLE &&
           model.generation == 2);

    assert(rawhid_app_ai_client_state_model_apply(&model, &reverse) ==
           RAWHID_APP_AI_CLIENT_HEARTBEAT);
    assert(model.generation == 2);

    struct rawhid_app_ai_client_state same_revision =
        state(RAWHID_APP_AI_ACTIVITY_COMPLETED, 1);
    assert(rawhid_app_ai_client_state_model_apply(&model, &same_revision) ==
           RAWHID_APP_AI_CLIENT_UPDATED_SAME_REVISION);
    assert(model.state.activity_state == RAWHID_APP_AI_ACTIVITY_COMPLETED &&
           model.generation == 3);

    assert(rawhid_app_ai_client_state_model_timeout(&model));
    assert(!model.valid && model.generation == 4);
    assert(!rawhid_app_ai_client_state_model_timeout(&model));

    assert(rawhid_app_ai_client_state_model_apply(&model, &same_revision) ==
           RAWHID_APP_AI_CLIENT_UPDATED);
    assert(model.valid && model.generation == 5);
    return 0;
}
