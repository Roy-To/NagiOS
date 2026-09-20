/* Generated from idl/application.nidl; do not edit by hand. */
#ifndef NAGI_SDK_H
#define NAGI_SDK_H

#include <stdint.h>

typedef struct { uint64_t value; } nagi_app_id_t;
typedef struct { uint64_t value; } nagi_app_session_id_t;
typedef struct { uint64_t value; } nagi_surface_id_t;
typedef struct { uint64_t value; } nagi_node_id_t;

typedef enum {
    NAGI_PRESENTATION_COMPACT = 0,
    NAGI_PRESENTATION_MEDIUM = 1,
    NAGI_PRESENTATION_EXPANDED = 2
} nagi_presentation_class_t;

typedef struct {
    uint16_t logical_width;
    uint16_t logical_height;
    uint16_t dpi;
    uint8_t touch;
    uint8_t keyboard;
    uint8_t pointer;
    nagi_presentation_class_t class_id;
} nagi_presentation_context_t;

typedef struct {
    nagi_app_id_t app_id;
    nagi_app_session_id_t session_id;
} nagi_application_t;

nagi_application_t nagi_application_open(nagi_app_id_t app_id, nagi_app_session_id_t session_id);
int nagi_application_surface(const nagi_application_t *application,
                             nagi_presentation_context_t context,
                             nagi_surface_id_t *surface_id,
                             nagi_node_id_t *node_id);

#endif
