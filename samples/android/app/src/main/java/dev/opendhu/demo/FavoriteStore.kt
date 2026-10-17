package dev.opendhu.demo

import android.content.Context

data class Place(val id: String, val name: String, val description: String)

object Places {
    val all = listOf(
        Place("harbour", "Harbour Cafe", "Coffee by the water"),
        Place("riverside", "Riverside Park", "Walk beside the river"),
        Place("library", "Central Library", "Books and a quiet corner"),
    )
}

class FavoriteStore(context: Context) {
    val preferences = context.applicationContext.getSharedPreferences("places", Context.MODE_PRIVATE)
    val favorite: Place?
        get() = Places.all.find { it.id == preferences.getString("favorite", null) }

    fun save(place: Place) {
        preferences.edit().putString("favorite", place.id).apply()
    }

    fun clear() {
        preferences.edit().remove("favorite").apply()
    }
}
