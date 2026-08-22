plugins {
    id("com.android.application") version "8.7.3"
}

android {
    namespace = "org.simnest.shell"
    compileSdk = 35

    defaultConfig {
        applicationId = "org.simnest.shell"
        minSdk = 28
        targetSdk = 35
        versionCode = 1
        versionName = "0.1.0"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    sourceSets.getByName("main").jniLibs.srcDir(rootProject.file("../../target/android-jniLibs"))
    testOptions.animationsDisabled = true
}

dependencies {
    androidTestImplementation("androidx.test:core-ktx:1.6.1")
    androidTestImplementation("androidx.test.ext:junit-ktx:1.2.1")
    androidTestImplementation("androidx.test:runner:1.6.2")
}
