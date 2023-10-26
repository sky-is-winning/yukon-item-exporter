/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at http://mozilla.org/MPL/2.0/. */
package {
import flash.display.MovieClip; public class Test extends MovieClip {}
}

import com.adobe.test.Assert;

//     var SECTION = "11.2.2.l2.as";
//     var VERSION = "ECMA_1";
//     var TITLE   = "The new operator";


    var testcases = getTestCases();
    
function getTestCases() {
    var array = new Array();
    var item = 0;

    var FUNCTION = new Function();

    f = new FUNCTION();
    array[item++] = Assert.expectEq( 
                                    "var FUNCTION = new Function(); f = new FUNCTION(); typeof f",
                                    "object",
                                    typeof f );
    return array;
}
