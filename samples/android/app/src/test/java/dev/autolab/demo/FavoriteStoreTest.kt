package dev.autolab.demo

import android.app.Application
import org.junit.Assert.*
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config

@RunWith(RobolectricTestRunner::class)
@Config(sdk = [35])
class FavoriteStoreTest {
    private lateinit var app: Application

    @Before fun reset() {
        app = RuntimeEnvironment.getApplication()
        app.getSharedPreferences("places", 0).edit().clear().commit()
    }

    @Test fun favoriteSurvivesRecreatingTheStore() {
        FavoriteStore(app).save(Places.all.first())
        assertEquals("Harbour Cafe", FavoriteStore(app).favorite?.name)
    }

    @Test fun clearingFavoritePersistsAndNotifiesTheOtherScreen() {
        val phone = FavoriteStore(app)
        val car = FavoriteStore(app)
        phone.save(Places.all.first())
        var observed = "not notified"
        val listener = android.content.SharedPreferences.OnSharedPreferenceChangeListener { _, _ ->
            observed = car.favorite?.name ?: "empty"
        }
        car.preferences.registerOnSharedPreferenceChangeListener(listener)
        phone.clear()
        assertNull(FavoriteStore(app).favorite)
        assertEquals("empty", observed)
        car.preferences.unregisterOnSharedPreferenceChangeListener(listener)
    }

    @Test fun staleStoredIdsDoNotBreakTheCatalog() {
        app.getSharedPreferences("places", 0).edit().putString("favorite", "removed-place").commit()
        assertNull(FavoriteStore(app).favorite)
    }
}
