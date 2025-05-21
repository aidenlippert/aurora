# ~/aurora_project/core_modules/cto/tests.py

# --- Imports from standard library typing module ---
from typing import TypeVar, Generic, Callable, Any, Set, Type

# --- Imports from your core cto_types module ---
# These are the crucial imports that provide access to your defined classes.
from .cto_types import Object, Morphism, BasicObject, BasicMorphism, Category

# --- Example Usage (Functional Tests) ---
# This block simulates how AURORA's components would interact using the CTO framework.
# It directly tests the concepts of Objects, Morphisms, Categories, Identity, and Composition.

if __name__ == "__main__":
    print("--- Running CTO Module Basic Tests ---")

    # 1. Define some objects representing Python types
    int_obj = BasicObject(int)
    str_obj = BasicObject(str)
    float_obj = BasicObject(float)

    # 2. Define some operations (functions that work on *instances* of these types)
    def int_to_str(x: int) -> str:
        return str(x)

    def str_to_len(s: str) -> int:
        return len(s)

    def int_to_float_half(x: int) -> float:
        return float(x) / 2.0

    # 3. Create morphisms from these operations and objects
    m_int_to_str = BasicMorphism(int_obj, str_obj, int_to_str)
    m_str_to_len = BasicMorphism(str_obj, int_obj, str_to_len)
    m_int_to_float = BasicMorphism(int_obj, float_obj, int_to_float_half)

    # 4. Apply a single morphism to an instance of its domain type
    result_str = m_int_to_str(123)
    print(f"\nApplying m_int_to_str to 123: '{result_str}' (type: {type(result_str).__name__})")
    assert result_str == "123"
    assert isinstance(result_str, str)

    # 5. Define a concrete Category to test composition
    # This category must implement the abstract methods from its base class 'Category'.
    class MyBasicCategory(Category[Type[Any], Any, Any]):
        def objects(self) -> Set[Object[Any]]:
            # Return the set of BasicObject instances that this category deals with
            return {int_obj, str_obj, float_obj}

        def morphisms(self, domain: Object[Any], codomain: Object[Any]) -> Set[Morphism[Any, Any]]:
            # This is a simplified registry; in a real system, this would be dynamic
            found_morphisms: Set[Morphism[Any, Any]] = set()
            if domain == int_obj and codomain == str_obj:
                found_morphisms.add(m_int_to_str)
            if domain == str_obj and codomain == int_obj:
                found_morphisms.add(m_str_to_len)
            if domain == int_obj and codomain == float_obj:
                found_morphisms.add(m_int_to_float)
            return found_morphisms

    my_cat = MyBasicCategory()

    # 6. Test Composition of Morphisms (m_int_to_str then m_str_to_len)
    # This chain: int -> str -> int
    try:
        m_int_to_len = my_cat.compose(m_int_to_str, m_str_to_len)
        final_result_int = m_int_to_len(4567) # Apply to an integer instance
        print(f"\nComposing m_int_to_str and m_str_to_len, then applying to 4567: {final_result_int} (type: {type(final_result_int).__name__})")
        assert final_result_int == 4 # len("4567") is 4
        assert isinstance(final_result_int, int)
    except Exception as e:
        print(f"\nError during int -> str -> int composition test: {e}")

    # 7. Test Identity Morphism
    identity_int_morphism = int_obj.identity()
    result_identity_int = identity_int_morphism(42)
    print(f"\nIdentity of int_obj applied to 42: {result_identity_int} (should be 42, type: {type(result_identity_int).__name__})")
    assert result_identity_int == 42
    assert isinstance(result_identity_int, int)

    identity_str_morphism = str_obj.identity()
    result_identity_str = identity_str_morphism("hello")
    print(f"Identity of str_obj applied to 'hello': '{result_identity_str}' (should be 'hello', type: {type(result_identity_str).__name__})")
    assert result_identity_str == "hello"
    assert isinstance(result_identity_str, str)

    # 8. Test Invalid Composition (Should raise a ValueError)
    print("\nAttempting invalid composition (int -> str followed by int -> float)...")
    try:
        # m_int_to_str output (str_obj) does not match m_int_to_float input (int_obj)
        my_cat.compose(m_int_to_str, m_int_to_float)
        print("ERROR: Invalid composition did NOT raise an exception!")
    except ValueError as e:
        print(f"Caught expected error for invalid composition: {e}")
        assert "Cannot compose morphisms" in str(e)
    except Exception as e:
        print(f"Caught unexpected error type for invalid composition: {type(e).__name__} - {e}")

    print("\n--- CTO Module Basic Tests Complete ---")