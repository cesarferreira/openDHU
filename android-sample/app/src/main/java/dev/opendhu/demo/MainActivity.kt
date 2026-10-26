package dev.opendhu.demo

import android.app.Activity
import android.content.SharedPreferences
import android.os.Bundle
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView

class MainActivity : Activity() {
    private lateinit var store: FavoriteStore
    private lateinit var favorite: TextView
    private val listener = SharedPreferences.OnSharedPreferenceChangeListener { _, _ -> showFavorite() }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)
        store = FavoriteStore(this)
        favorite = findViewById(R.id.favorite)
        val choices = findViewById<LinearLayout>(R.id.place_choices)
        Places.all.forEach { place ->
            choices.addView(Button(this).apply {
                text = place.name
                isAllCaps = false
                setOnClickListener { store.save(place) }
            })
        }
        findViewById<Button>(R.id.reset).setOnClickListener { store.clear() }
        val content = findViewById<android.view.View>(R.id.content)
        val padding = (24 * resources.displayMetrics.density).toInt()
        @Suppress("DEPRECATION")
        content.setOnApplyWindowInsetsListener { view, insets ->
            view.setPadding(padding + insets.systemWindowInsetLeft, padding + insets.systemWindowInsetTop,
                padding + insets.systemWindowInsetRight, padding + insets.systemWindowInsetBottom)
            insets
        }
    }

    override fun onStart() {
        super.onStart()
        store.preferences.registerOnSharedPreferenceChangeListener(listener)
        showFavorite()
    }

    override fun onStop() {
        store.preferences.unregisterOnSharedPreferenceChangeListener(listener)
        super.onStop()
    }

    private fun showFavorite() {
        favorite.text = store.favorite?.name ?: getString(R.string.no_favorite)
    }
}
