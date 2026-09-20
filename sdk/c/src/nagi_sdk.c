/* Generated from idl/application.nidl; do not edit by hand. */
#include "nagi_sdk.h"

nagi_application_t nagi_application_open(nagi_app_id_t app_id,
                                         nagi_app_session_id_t session_id) {
    nagi_application_t application = { app_id, session_id };
    return application;
}

int nagi_application_surface(const nagi_application_t *application,
                             nagi_presentation_context_t context,
                             nagi_surface_id_t *surface_id,
                             nagi_node_id_t *node_id) {
    if (application == 0 || surface_id == 0 || node_id == 0 ||
        context.logical_width == 0 || context.logical_height == 0) {
        return -1;
    }
    surface_id->value = application->session_id.value;
    node_id->value = 1;
    return 0;
}
