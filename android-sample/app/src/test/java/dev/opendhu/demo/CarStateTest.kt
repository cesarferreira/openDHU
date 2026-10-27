package dev.opendhu.demo

import androidx.car.app.model.ItemList
import androidx.car.app.model.ListTemplate
import androidx.car.app.model.Row
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class CarStateTest {
    @Test fun listTemplateIsDescribedWithTitleAndRows() {
        val template = ListTemplate.Builder().setTitle("Places").setSingleList(
            ItemList.Builder()
                .addItem(Row.Builder().setTitle("Harbour Cafe").setOnClickListener {}.build())
                .addItem(Row.Builder().setTitle("About").build())
                .build(),
        ).build()

        CarState.record(template)
        val json = org.json.JSONObject(CarState.json)

        assertEquals("ListTemplate", json.getString("type"))
        assertEquals("Places", json.getString("title"))
        val rows = json.getJSONArray("rows")
        assertEquals("Harbour Cafe", rows.getJSONObject(0).getString("title"))
        assertTrue(rows.getJSONObject(0).getBoolean("clickable"))
        assertFalse(rows.getJSONObject(1).getBoolean("clickable"))
    }
}
