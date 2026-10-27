package dev.opendhu.demo

import androidx.car.app.model.ListTemplate
import androidx.car.app.model.Row
import androidx.car.app.model.Template
import org.json.JSONArray
import org.json.JSONObject

/**
 * The template the car screen was last given, as JSON. Android Auto draws templates into windows
 * that accessibility cannot read, so tests read this (through the debug-only [CarStateProvider])
 * instead of OCR-ing the projected frame.
 */
object CarState {
    @Volatile
    var json: String = "{}"
        private set

    fun record(template: Template) {
        json = describe(template).toString()
    }

    fun describe(template: Template): JSONObject {
        val out = JSONObject().put("type", template::class.java.simpleName)
        if (template is ListTemplate) {
            out.put("title", template.title?.toString())
            val rows = JSONArray()
            template.singleList?.items?.filterIsInstance<Row>()?.forEach { row ->
                rows.put(
                    JSONObject()
                        .put("title", row.title?.toString())
                        .put("clickable", row.onClickDelegate != null),
                )
            }
            out.put("rows", rows)
        }
        return out
    }
}
