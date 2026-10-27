package dev.opendhu.demo

import android.content.SharedPreferences
import androidx.car.app.CarContext
import androidx.car.app.Screen
import androidx.car.app.model.Action
import androidx.car.app.model.ItemList
import androidx.car.app.model.ListTemplate
import androidx.car.app.model.Row
import androidx.car.app.model.Template
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner

/** Records every template it hands to the host, for [CarState]. */
abstract class DemoScreen(context: CarContext) : Screen(context) {
    final override fun onGetTemplate(): Template = buildTemplate().also(CarState::record)
    protected abstract fun buildTemplate(): Template
}

abstract class FavoriteScreen(context: CarContext) : DemoScreen(context) {
    protected val store = FavoriteStore(context)
    private val listener = SharedPreferences.OnSharedPreferenceChangeListener { _, _ -> invalidate() }

    init {
        lifecycle.addObserver(object : DefaultLifecycleObserver {
            override fun onStart(owner: LifecycleOwner) {
                store.preferences.registerOnSharedPreferenceChangeListener(listener)
                invalidate()
            }
            override fun onStop(owner: LifecycleOwner) {
                store.preferences.unregisterOnSharedPreferenceChangeListener(listener)
            }
        })
    }
}

class HomeScreen(context: CarContext) : FavoriteScreen(context) {
    override fun buildTemplate(): Template {
        val favorite = store.favorite
        val savedRow = Row.Builder()
            .setTitle(favorite?.let { carContext.getString(R.string.saved_place, it.name) }
                ?: carContext.getString(R.string.no_favorite))
        if (favorite != null) {
            savedRow.setOnClickListener { screenManager.push(PlaceScreen(carContext, favorite)) }
        }
        val rows = ItemList.Builder()
            .addItem(Row.Builder().setTitle(carContext.getString(R.string.browse_places))
                .setOnClickListener { screenManager.push(PlacesScreen(carContext)) }.build())
            .addItem(savedRow.build())
            .addItem(Row.Builder().setTitle(carContext.getString(R.string.reset_favorite))
                .setOnClickListener { store.clear(); invalidate() }.build())
            .build()
        return ListTemplate.Builder().setTitle(carContext.getString(R.string.app_name))
            .setHeaderAction(Action.APP_ICON).setSingleList(rows).build()
    }
}

class PlacesScreen(context: CarContext) : DemoScreen(context) {
    override fun buildTemplate(): Template {
        val rows = ItemList.Builder()
        Places.all.forEach { place ->
            rows.addItem(Row.Builder().setTitle(place.name)
                .setOnClickListener { screenManager.push(PlaceScreen(carContext, place)) }.build())
        }
        return ListTemplate.Builder().setTitle(carContext.getString(R.string.places_title))
            .setHeaderAction(Action.BACK).setSingleList(rows.build()).build()
    }
}

class PlaceScreen(context: CarContext, private val place: Place) : FavoriteScreen(context) {
    override fun buildTemplate(): Template {
        val rows = ItemList.Builder()
            .addItem(Row.Builder().setTitle(place.description).build())
            .addItem(Row.Builder().setTitle(carContext.getString(R.string.save_favorite))
                .setOnClickListener {
                    store.save(place)
                    screenManager.popToRoot()
                }.build())
            .addItem(Row.Builder().setTitle(carContext.getString(R.string.back_to_places))
                .setOnClickListener {
                    if (screenManager.stackSize > 2) screenManager.pop()
                    else {
                        screenManager.popToRoot()
                        screenManager.push(PlacesScreen(carContext))
                    }
                }.build())
            .build()
        return ListTemplate.Builder().setTitle(place.name).setHeaderAction(Action.BACK)
            .setSingleList(rows).build()
    }
}
